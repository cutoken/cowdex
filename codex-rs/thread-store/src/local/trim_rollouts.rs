use std::fs::File;
use std::fs::OpenOptions;
use std::fs::{self};
use std::io::BufRead;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::io::{self};
use std::path::Path;

use codex_protocol::ThreadId;
use codex_protocol::protocol::ThreadHistoryMode;
use codex_rollout::ModelContextScan;
use codex_rollout::ModelContextScanProgress;
use codex_rollout::ReverseJsonlScanner;
use codex_rollout::RolloutItem;
use codex_rollout::RolloutLine;
use codex_rollout::RolloutReferenceIndex;
use codex_rollout::ScanOutcome;

use super::LocalThreadStore;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;

/// Result of replacing inactive rollouts with their resumable suffixes.
#[derive(Debug, Default)]
pub struct RolloutTrimReport {
    pub trimmed: usize,
    pub reclaimed_bytes: u64,
    pub skipped: usize,
}

impl LocalThreadStore {
    /// Back up and atomically trim inactive, unreferenced paginated rollouts.
    pub async fn trim_rollouts(&self) -> ThreadStoreResult<RolloutTrimReport> {
        let index = RolloutReferenceIndex::scan(&self.config.codex_home)
            .await
            .map_err(trim_error)?;
        let mut report = RolloutTrimReport::default();
        for (rollout_id, thread_id, path) in index.rollouts() {
            if path.extension().is_none_or(|ext| ext != "jsonl")
                || tokio::fs::metadata(path).await.map_err(trim_error)?.len() < 1024 * 1024
                || self.live_recorders.lock().await.contains_key(&thread_id)
            {
                continue;
            }
            let _lifecycle = self.live_writer_locks.lock_lifecycle(thread_id).await;
            let _writer = self.live_writer_locks.lock(thread_id).await;
            let Ok(_lock) = self.writer_lock_coordinator.acquire(thread_id) else {
                report.skipped += 1;
                continue;
            };
            // Recheck references after excluding writers and new fork snapshots.
            let current = RolloutReferenceIndex::scan(&self.config.codex_home)
                .await
                .map_err(trim_error)?;
            if current.reference_count(rollout_id) != 0 {
                report.skipped += 1;
                continue;
            }
            let path = path.to_path_buf();
            let backup = self.config.codex_home.join("backups/rollouts");
            let candidate = path.with_extension(format!("trim-{}", ThreadId::new()));
            let selection = select_suffix(&path).map_err(trim_error)?;
            let Some(lines) = selection else {
                report.skipped += 1;
                continue;
            };
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
                .map_err(trim_error)?;
            output
                .set_permissions(fs::metadata(&path).map_err(trim_error)?.permissions())
                .map_err(trim_error)?;
            for (ordinal, mut line) in lines.into_iter().enumerate() {
                line["ordinal"] = serde_json::json!(ordinal);
                serde_json::to_writer(&mut output, &line).map_err(trim_error)?;
                output.write_all(b"\n").map_err(trim_error)?;
            }
            output.sync_all().map_err(trim_error)?;
            drop(output);
            let before = fs::metadata(&path).map_err(trim_error)?;
            let after = fs::metadata(&candidate).map_err(trim_error)?.len();
            // Verify through the same bounded selector used by resume, before touching the source.
            let verified = select_suffix(&candidate).map_err(trim_error)?;
            if after >= before.len()
                || verified.is_none()
                || context_json(&path)? != context_json(&candidate)?
            {
                fs::remove_file(&candidate).map_err(trim_error)?;
                report.skipped += 1;
                continue;
            }
            fs::create_dir_all(&backup).map_err(trim_error)?;
            let backup = backup.join(format!(
                "{}-{}.zst",
                path.file_name().unwrap().to_string_lossy(),
                ThreadId::new()
            ));
            let source = path.clone();
            tokio::task::spawn_blocking(move || -> io::Result<()> {
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&backup)?;
                file.set_permissions(fs::metadata(&source)?.permissions())?;
                let mut encoder =
                    zstd::stream::write::Encoder::new(BufWriter::new(file), /*level*/ 1)?;
                io::copy(&mut File::open(source)?, &mut encoder)?;
                let mut writer = encoder.finish()?;
                writer.flush()?;
                writer.get_ref().sync_all()?;
                #[cfg(unix)]
                File::open(backup.parent().unwrap())?.sync_all()?;
                Ok(())
            })
            .await
            .map_err(trim_error)?
            .map_err(trim_error)?;
            // Old byte offsets/ordinals must never survive the replacement in the UI cache.
            super::thread_history::delete_thread(self, rollout_id).await?;
            let file = OpenOptions::new()
                .write(true)
                .open(&candidate)
                .map_err(trim_error)?;
            file.set_times(
                fs::FileTimes::new().set_modified(before.modified().map_err(trim_error)?),
            )
            .map_err(trim_error)?;
            drop(file);
            fs::rename(&candidate, &path).map_err(trim_error)?;
            #[cfg(unix)]
            File::open(path.parent().unwrap())
                .and_then(|dir| dir.sync_all())
                .map_err(trim_error)?;
            report.trimmed += 1;
            report.reclaimed_bytes += before.len() - after;
        }
        Ok(report)
    }
}

// Preserve every record in the selected suffix, including timestamps and turn/configuration
// metadata. Unsupported, partial or malformed suffixes are never rewritten.
fn select_suffix(path: &Path) -> io::Result<Option<Vec<serde_json::Value>>> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() == 0 {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0];
    std::io::Read::read_exact(&mut file, &mut last)?;
    if last != [b'\n'] {
        return Ok(None);
    }
    file.rewind()?;
    let mut head = String::new();
    BufReader::new(&file).read_line(&mut head)?;
    let header: serde_json::Value = serde_json::from_str(&head)?;
    let parsed_header: RolloutLine = serde_json::from_value(header.clone())?;
    let RolloutItem::SessionMeta(meta) = &parsed_header.item else {
        return Ok(None);
    };
    if meta.meta.history_mode != ThreadHistoryMode::Paginated
        || meta.meta.history_base.is_some()
        || meta.meta.subagent_history_start_ordinal.is_some()
    {
        return Ok(None);
    }
    let mut scanner = ReverseJsonlScanner::new(file)?;
    let mut scan = ModelContextScan::default();
    let mut retained = Vec::new();
    let mut bytes = 0;
    while let Some(outcome) = scanner.scan_next::<serde_json::Value>()? {
        let ScanOutcome::Parsed(line) = outcome else {
            return Ok(None);
        };
        if line["type"] == "session_meta" {
            return Ok(None);
        }
        bytes += serde_json::to_vec(&line)?.len();
        if bytes > 256 * 1024 * 1024 {
            return Ok(None);
        }
        // Resume skips records from older or newer event schemas. Preserve those records
        // verbatim in the suffix, but only feed supported records to its cutoff selector.
        let progress = serde_json::from_value::<RolloutLine>(line.clone())
            .map_or(ModelContextScanProgress::Continue, |parsed| {
                scan.push(parsed.item)
            });
        retained.push(line);
        if progress == ModelContextScanProgress::Complete {
            retained.push(header);
            retained.reverse();
            return Ok(Some(retained));
        }
    }
    Ok(None)
}

fn context_json(path: &Path) -> ThreadStoreResult<serde_json::Value> {
    let lines = select_suffix(path)
        .map_err(trim_error)?
        .ok_or_else(|| trim_error("no safe checkpoint"))?;
    serde_json::to_value(
        lines
            .into_iter()
            .filter_map(|line| {
                serde_json::from_value::<RolloutLine>(line)
                    .ok()
                    .map(|parsed| parsed.item)
            })
            .collect::<Vec<_>>(),
    )
    .map_err(trim_error)
}

fn trim_error(error: impl std::fmt::Display) -> ThreadStoreError {
    ThreadStoreError::Internal {
        message: format!("rollout trimming: {error}"),
    }
}
