use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::Datelike;
use chrono::Days;
use chrono::Local;
use chrono::NaiveDate;
use clap::Parser;
use codex_http_client::HttpClientFactory;
use codex_http_client::OutboundProxyPolicy;
use codex_login::CODEX_ACCESS_TOKEN_ENV_VAR;
use codex_login::CODEX_API_KEY_ENV_VAR;
use codex_login::OPENAI_API_KEY_ENV_VAR;
use codex_protocol::shell_environment::OPENAI_FEDERATION_RULE_ID_ENV_VAR;
use codex_protocol::shell_environment::OPENAI_IDENTITY_TOKEN_FILE_ENV_VAR;
use codex_protocol::shell_environment::OPENAI_WORKLOAD_IDENTITY_CONTEXT_ENV_VAR;
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const AUTH_ENV_VARS: [&str; 6] = [
    OPENAI_API_KEY_ENV_VAR,
    CODEX_API_KEY_ENV_VAR,
    CODEX_ACCESS_TOKEN_ENV_VAR,
    OPENAI_FEDERATION_RULE_ID_ENV_VAR,
    OPENAI_IDENTITY_TOKEN_FILE_ENV_VAR,
    OPENAI_WORKLOAD_IDENTITY_CONTEXT_ENV_VAR,
];

const FILE_AUTH_OVERRIDE: &str = r#"cli_auth_credentials_store="file""#;
const LIVE_SEARCH_OVERRIDE: &str = r#"web_search="live""#;

#[derive(Debug, Parser)]
pub(crate) struct KeepwarmCommand {
    /// Use the device-code login flow instead of opening a browser.
    #[arg(long = "device-auth", default_value_t = false)]
    device_auth: bool,
}

#[derive(Clone, Copy)]
enum LoginFlow {
    Browser,
    DeviceCode,
}

#[derive(Debug)]
pub(crate) struct KeepwarmReport {
    report_path: PathBuf,
    account_email: Option<String>,
}

impl fmt::Display for KeepwarmReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Research report saved to {}", self.report_path.display())?;
        match &self.account_email {
            Some(email) => write!(f, "{email} account is refreshed."),
            None => write!(f, "Account is refreshed (email unavailable)."),
        }
    }
}

pub(crate) async fn run(command: KeepwarmCommand) -> Result<KeepwarmReport> {
    let executable = std::env::current_exe().context("failed to locate the Cowdex executable")?;
    let temp_root = std::env::temp_dir();
    let report_dir = temp_root.join("stocks");

    run_with(
        &executable,
        &temp_root,
        &report_dir,
        Local::now().date_naive(),
        if command.device_auth {
            LoginFlow::DeviceCode
        } else {
            LoginFlow::Browser
        },
        run_supervised_child,
    )
    .await
}

async fn run_with<RunChild, ChildFuture>(
    executable: &Path,
    temp_root: &Path,
    report_dir: &Path,
    today: NaiveDate,
    login_flow: LoginFlow,
    mut run_child: RunChild,
) -> Result<KeepwarmReport>
where
    RunChild: FnMut(Command) -> ChildFuture,
    ChildFuture: Future<Output = Result<bool>>,
{
    let isolated_home = tempfile::Builder::new()
        .prefix("cowdex-keepwarm-")
        .tempdir_in(temp_root)
        .context("failed to create an isolated Cowdex home")?;

    let mut login = isolated_command(executable, isolated_home.path());
    login.args(["-c", FILE_AUTH_OVERRIDE, "login"]);
    if matches!(login_flow, LoginFlow::DeviceCode) {
        login.arg("--device-auth");
    }
    if !run_child(login).await? {
        bail!("temporary account login failed; the existing Cowdex login was not changed");
    }

    fs::create_dir_all(report_dir)
        .with_context(|| format!("failed to create report directory {}", report_dir.display()))?;
    let report_path = reserve_report_path(report_dir, today)?;

    let mut research = isolated_command(executable, isolated_home.path());
    research.args([
        "-c",
        FILE_AUTH_OVERRIDE,
        "-c",
        LIVE_SEARCH_OVERRIDE,
        "exec",
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--output-last-message",
    ]);
    research.arg(&report_path).arg(research_prompt(today));
    research.stdout(Stdio::null()).stderr(Stdio::null());

    eprintln!("Refreshing account...");
    let research_succeeded = run_child(research).await;
    match research_succeeded {
        Ok(true)
            if report_path
                .metadata()
                .is_ok_and(|metadata| metadata.len() > 0) =>
        {
            let account_email = codex_login::CodexAuth::from_auth_storage(
                isolated_home.path(),
                codex_login::AuthCredentialsStoreMode::File,
                /*chatgpt_base_url*/ None,
                codex_login::AuthKeyringBackendKind::default(),
                &codex_login::AuthRouteConfig::from_http_client_factory(HttpClientFactory::new(
                    OutboundProxyPolicy::ReqwestDefault,
                )),
            )
            .await
            .ok()
            .flatten()
            .and_then(|auth| auth.get_account_email());
            Ok(KeepwarmReport {
                report_path,
                account_email,
            })
        }
        Ok(true) => {
            remove_incomplete_report(&report_path);
            bail!("research completed without producing a report");
        }
        Ok(false) => {
            remove_incomplete_report(&report_path);
            bail!("research failed; no report was saved");
        }
        Err(error) => {
            remove_incomplete_report(&report_path);
            Err(error)
        }
    }
}

async fn run_supervised_child(command: Command) -> Result<bool> {
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .spawn()
        .context("failed to start an isolated Cowdex child process")?;

    tokio::select! {
        status = child.wait() => {
            Ok(status.context("failed to wait for an isolated Cowdex child process")?.success())
        }
        interrupt = tokio::signal::ctrl_c() => {
            let kill_result = child.start_kill();
            let _ = child.wait().await;
            interrupt.context("failed to listen for Ctrl-C")?;
            kill_result.context("failed to stop the isolated Cowdex child process")?;
            bail!("keepwarm interrupted; temporary credentials and incomplete output were removed")
        }
    }
}

fn isolated_command(executable: &Path, isolated_home: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .current_dir(isolated_home)
        .env("CODEX_HOME", isolated_home)
        .env(codex_state::SQLITE_HOME_ENV, isolated_home);
    for name in AUTH_ENV_VARS {
        command.env_remove(name);
    }
    command
}

fn reserve_report_path(report_dir: &Path, today: NaiveDate) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = format!(
        "googl-gld-{}-{}-{nonce}",
        today.format("%Y-%m-%d"),
        std::process::id()
    );

    for suffix in 0..1000 {
        let suffix = if suffix == 0 {
            String::new()
        } else {
            format!("-{suffix}")
        };
        let path = report_dir.join(format!("{stem}{suffix}.md"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to reserve report file {}", path.display()));
            }
        }
    }

    bail!(
        "failed to reserve a unique report file in {}",
        report_dir.display()
    )
}

fn remove_incomplete_report(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!(
            "warning: failed to remove incomplete report {}: {error}",
            path.display()
        );
    }
}

fn research_prompt(today: NaiveDate) -> String {
    let days_since_monday = u64::from(today.weekday().num_days_from_monday());
    let monday = today - Days::new(days_since_monday) + Days::new(/*num*/ 7);
    let friday = monday + Days::new(/*num*/ 4);

    format!(
        r#"Today is {today}. Research the market prospects for Alphabet Class A (NASDAQ: GOOGL) and SPDR Gold Shares (NYSE Arca: GLD) for next calendar week, Monday {monday} through Friday {friday}.

Use live web search and current, reputable sources. Verify the relevant US trading dates and any market holidays. Write a self-contained Markdown report covering recent price and performance context, confirmed company/fund and macro catalysts scheduled for that week, material news and risks, and the factors most likely to move each instrument. Include balanced bull, base, and bear scenarios for each, explain the GOOGL/GLD cross-asset relationship where useful, and state what evidence during the week would strengthen or weaken each scenario.

Distinguish sourced facts from your own inference. Do not fabricate prices, events, forecasts, or citations, and do not imply certainty about future returns. Include numeric forecasts only when attributed to a named source or transparently calculated from cited values. Give direct source links next to the claims they support, with publisher and publication date when available, and end with a compact source list plus any important data gaps or conflicting evidence."#
    )
}

#[cfg(test)]
#[path = "keepwarm_tests.rs"]
mod tests;
