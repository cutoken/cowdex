use std::fs::OpenOptions;
use std::fs::{self};
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::time::Duration;
use std::time::SystemTime;

use clap::Args;
use codex_core::config::ConfigBuilder;
use codex_thread_store::LocalThreadStore;
use codex_thread_store::LocalThreadStoreConfig;
use codex_utils_cli::CliConfigOverrides;

#[derive(Debug, Args)]
pub(crate) struct CleanupCommand {
    #[arg(long, hide = true)]
    automatic: bool,
}

pub(crate) async fn run(
    command: CleanupCommand,
    overrides: CliConfigOverrides,
) -> anyhow::Result<()> {
    let config = ConfigBuilder::default()
        .cli_overrides(overrides.parse_overrides().map_err(anyhow::Error::msg)?)
        .build()
        .await?;
    let stamp = config.codex_home.join("cleanup.last");
    if command.automatic
        && fs::metadata(&stamp)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(6 * 60 * 60))
    {
        return Ok(());
    }
    fs::create_dir_all(config.codex_home.as_path())?;
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.codex_home.join("cleanup.lock"))?;
    if guard.try_lock().is_err() {
        return Ok(());
    }
    let state = codex_rollout::state_db::try_init(&config).await?;
    let store = LocalThreadStore::new(
        LocalThreadStoreConfig::from_config(&config),
        Some(state.clone()),
    );
    let stale = store.cleanup_stale_subagents().await?;
    let trimmed = if command.automatic {
        Default::default()
    } else {
        store.trim_rollouts().await?
    };
    let logs = state.cleanup_logs().await?;
    // Legacy text logs are optional. Retain only a small tail, without unlinking live files.
    if let Ok(entries) = fs::read_dir(&config.log_dir) {
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() || entry.file_name() != "codex-tui.log" {
                continue;
            }
            let meta = entry.metadata()?;
            if SystemTime::now()
                .duration_since(meta.modified()?)
                .unwrap_or_default()
                > Duration::from_secs(7 * 24 * 60 * 60)
            {
                fs::remove_file(path)?;
            } else if meta.len() > 8 * 1024 * 1024 {
                let mut file = OpenOptions::new().read(true).write(true).open(path)?;
                file.seek(SeekFrom::End(-1_048_576))?;
                let mut tail = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut tail)?;
                file.rewind()?;
                file.write_all(&tail)?;
                file.set_len(tail.len() as u64)?;
            }
        }
    }
    store.trim_history_cache().await?;
    fs::write(stamp, b"")?;
    if !command.automatic {
        println!(
            "{}",
            report(
                stale.removed_sessions,
                trimmed.trimmed,
                trimmed.reclaimed_bytes,
                logs,
                trimmed.skipped
            )
        );
    }
    Ok(())
}

fn report(removed: usize, trimmed: usize, bytes: u64, logs: u64, skipped: usize) -> String {
    format!(
        "Cleanup: removed {removed} expired subagent sessions; trimmed {trimmed} rollouts ({:.1} MiB); removed {logs} log records. Kept {skipped} rollouts in use, referenced, or without a safe checkpoint. Originals: backups/rollouts under Cowdex home.",
        bytes as f64 / 1_048_576.0
    )
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
