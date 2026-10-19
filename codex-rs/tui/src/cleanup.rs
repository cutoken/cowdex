use crate::legacy_core::config::Config;
use tokio::process::Command;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::history_cell;

pub(crate) fn spawn(config: &Config, events: Option<AppEventSender>) {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let mut command = Command::new(executable);
    command
        .arg("-c")
        .arg(format!(
            "sqlite_home={}",
            toml::Value::String(config.sqlite_config().home().display().to_string())
        ))
        .arg("-c")
        .arg(format!(
            "log_dir={}",
            toml::Value::String(config.log_dir.display().to_string())
        ))
        .arg("cleanup")
        .env("CODEX_HOME", config.codex_home.as_path());
    if events.is_none() {
        command.arg("--automatic");
    }
    tokio::spawn(async move {
        let result = command.output().await;
        if let Some(events) = events {
            let cell = match result {
                Ok(output) if output.status.success() => history_cell::new_info_event(
                    String::from_utf8_lossy(&output.stdout).trim().to_string(),
                    /*hint*/ None,
                ),
                Ok(output) => history_cell::new_error_event(format!(
                    "Cleanup failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )),
                Err(error) => history_cell::new_error_event(format!("Cleanup failed: {error}")),
            };
            events.send(AppEvent::InsertHistoryCell(Box::new(cell)));
        }
    });
}
