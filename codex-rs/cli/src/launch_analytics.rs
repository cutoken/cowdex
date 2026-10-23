//! One background launch event per installation per UTC day.

use std::fs::OpenOptions;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use codex_http_client::ClientRouteClass;
use codex_http_client::HttpClientFactory;
use codex_http_client::OutboundProxyPolicy;
use codex_protocol::ThreadId;
use serde::Deserialize;
use serde::Serialize;

const ENDPOINT: &str = "https://cowdex-analytics.agentcron.ai/launch";

#[derive(Deserialize, Serialize)]
struct LaunchState {
    installation_id: String,
    last_day: Option<i64>,
    version: String,
}

pub(super) fn record_launch() {
    let Ok(codex_home) = codex_core::config::find_codex_home() else {
        return;
    };
    tokio::spawn(async move {
        let _ = send_daily_launch(
            &codex_home,
            ENDPOINT,
            chrono::Utc::now().timestamp(),
            env!("CARGO_PKG_VERSION"),
        )
        .await;
    });
}

async fn send_daily_launch(
    codex_home: &Path,
    endpoint: &str,
    now: i64,
    version: &str,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(codex_home)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(codex_home.join("cowdex-analytics.json"))?;
    // Holding this lock through the request prevents simultaneous terminals counting twice.
    if file.try_lock().is_err() {
        return Ok(());
    }
    let mut state: LaunchState =
        serde_json::from_reader((&mut file).take(1024)).unwrap_or_else(|_| LaunchState {
            installation_id: ThreadId::new().to_string(),
            last_day: None,
            version: String::new(),
        });
    let day = now.div_euclid(86_400);
    if state.last_day == Some(day) {
        return Ok(());
    }
    let client = HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault)
        .build_client_without_request_logging(endpoint, ClientRouteClass::Other)?;
    let response = client
        .post(endpoint)
        .timeout(Duration::from_secs(/*secs*/ 3))
        .json(&serde_json::json!({
            "event": "launch",
            "installation_id": state.installation_id,
            "version": version,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "first_launch": state.last_day.is_none(),
            "updated": state.last_day.is_some() && state.version != version,
        }))
        .send()
        .await?;
    response.error_for_status()?;
    // Offline launches can retry next time; a failed ping must not consume the daily allowance.
    state.last_day = Some(day);
    state.version = version.to_string();
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(&serde_json::to_vec(&state)?)?;
    Ok(())
}

#[cfg(test)]
#[path = "launch_analytics_tests.rs"]
mod tests;
