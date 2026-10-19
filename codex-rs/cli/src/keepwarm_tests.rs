use super::*;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::ffi::OsString;

#[derive(Debug, PartialEq, Eq)]
struct ObservedCommand {
    program: OsString,
    args: Vec<OsString>,
    current_dir: Option<PathBuf>,
    env: BTreeMap<OsString, Option<OsString>>,
}

impl ObservedCommand {
    fn capture(command: &Command) -> Self {
        Self {
            program: command.get_program().to_os_string(),
            args: command.get_args().map(OsStr::to_os_string).collect(),
            current_dir: command.get_current_dir().map(Path::to_path_buf),
            env: command
                .get_envs()
                .map(|(name, value)| {
                    (
                        name.to_os_string(),
                        value.map(std::ffi::OsStr::to_os_string),
                    )
                })
                .collect(),
        }
    }
}

#[tokio::test]
async fn successful_run_isolates_both_children_and_keeps_only_the_report() -> Result<()> {
    let temp_root = tempfile::tempdir()?;
    let report_dir = temp_root.path().join("stocks");
    let executable = temp_root.path().join("cowdex-test");
    let today =
        NaiveDate::from_ymd_opt(/*year*/ 2026, /*month*/ 9, /*day*/ 12).expect("valid date");
    let mut observed = Vec::new();
    let mut isolated_home = None;

    let report = run_with(
        &executable,
        temp_root.path(),
        &report_dir,
        today,
        LoginFlow::Browser,
        |command| {
            observed.push(ObservedCommand::capture(&command));
            isolated_home.get_or_insert_with(|| {
                command
                    .get_current_dir()
                    .expect("isolated current directory")
                    .to_path_buf()
            });
            if observed.len() == 1 {
                let auth = serde_json::json!({
                    "auth_mode": "chatgpt",
                    "tokens": {
                        "id_token": "eyJhbGciOiJub25lIn0.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL3Byb2ZpbGUiOnsiZW1haWwiOiJrZWVwd2FybUBleGFtcGxlLmNvbSJ9fQ.c2ln",
                        "access_token": "test-access-token",
                        "refresh_token": "test-refresh-token"
                    }
                });
                if let Err(error) = fs::write(
                    command.get_current_dir().unwrap().join("auth.json"),
                    auth.to_string(),
                ) {
                    return std::future::ready(Err(error.into()));
                }
            }
            if observed.len() == 2 {
                let args = command.get_args().collect::<Vec<_>>();
                let output_index = args
                    .iter()
                    .position(|arg| *arg == OsStr::new("--output-last-message"))
                    .expect("output flag");
                if let Err(error) = fs::write(Path::new(args[output_index + 1]), "# report\n") {
                    return std::future::ready(Err(error.into()));
                }
            }
            std::future::ready(Ok(true))
        },
    )
    .await?;

    let report_path = &report.report_path;
    let isolated_home = isolated_home.expect("isolated home captured");
    let removed_env: BTreeMap<OsString, Option<OsString>> = AUTH_ENV_VARS
        .into_iter()
        .map(|name| (OsString::from(name), None))
        .chain([
            (
                OsString::from("CODEX_HOME"),
                Some(isolated_home.as_os_str().to_os_string()),
            ),
            (
                OsString::from(codex_state::SQLITE_HOME_ENV),
                Some(isolated_home.as_os_str().to_os_string()),
            ),
        ])
        .collect();
    let expected = vec![
        ObservedCommand {
            program: executable.as_os_str().to_os_string(),
            args: ["-c", FILE_AUTH_OVERRIDE, "login"]
                .into_iter()
                .map(OsString::from)
                .collect(),
            current_dir: Some(isolated_home.clone()),
            env: removed_env.clone(),
        },
        ObservedCommand {
            program: executable.as_os_str().to_os_string(),
            args: [
                OsString::from("-c"),
                OsString::from(FILE_AUTH_OVERRIDE),
                OsString::from("-c"),
                OsString::from(LIVE_SEARCH_OVERRIDE),
                OsString::from("exec"),
                OsString::from("--ephemeral"),
                OsString::from("--ignore-user-config"),
                OsString::from("--ignore-rules"),
                OsString::from("--skip-git-repo-check"),
                OsString::from("--sandbox"),
                OsString::from("read-only"),
                OsString::from("--output-last-message"),
                report_path.as_os_str().to_os_string(),
                OsString::from(research_prompt(today)),
            ]
            .into_iter()
            .collect(),
            current_dir: Some(isolated_home.clone()),
            env: removed_env,
        },
    ];

    assert_eq!(observed, expected);
    assert_eq!(fs::read_to_string(report_path)?, "# report\n");
    assert_eq!(
        report.account_email.as_deref(),
        Some("keepwarm@example.com")
    );
    insta::assert_snapshot!(
        report
            .to_string()
            .replace(report_path.to_str().unwrap(), "<REPORT_PATH>")
    );
    assert!(!isolated_home.exists());
    Ok(())
}

#[tokio::test]
async fn failed_login_stops_before_allocating_a_report() -> Result<()> {
    let temp_root = tempfile::tempdir()?;
    let report_dir = temp_root.path().join("stocks");
    let mut calls = 0;
    let mut isolated_home = None;

    let error = run_with(
        Path::new("cowdex-test"),
        temp_root.path(),
        &report_dir,
        NaiveDate::from_ymd_opt(/*year*/ 2026, /*month*/ 9, /*day*/ 12).expect("valid date"),
        LoginFlow::DeviceCode,
        |command| {
            calls += 1;
            isolated_home = command.get_current_dir().map(Path::to_path_buf);
            assert!(command.get_args().any(|arg| arg == "--device-auth"));
            std::future::ready(Ok(false))
        },
    )
    .await
    .expect_err("login should fail");

    assert_eq!(calls, 1);
    assert!(
        error
            .to_string()
            .contains("existing Cowdex login was not changed")
    );
    assert!(!report_dir.exists());
    assert!(!isolated_home.expect("isolated home captured").exists());
    Ok(())
}

#[tokio::test]
async fn failed_research_removes_the_partial_report_and_temporary_login() -> Result<()> {
    let temp_root = tempfile::tempdir()?;
    let report_dir = temp_root.path().join("stocks");
    let mut calls = 0;
    let mut isolated_home = None;

    let error = run_with(
        Path::new("cowdex-test"),
        temp_root.path(),
        &report_dir,
        NaiveDate::from_ymd_opt(/*year*/ 2026, /*month*/ 9, /*day*/ 12).expect("valid date"),
        LoginFlow::Browser,
        |command| {
            calls += 1;
            isolated_home = command.get_current_dir().map(Path::to_path_buf);
            if calls == 2 {
                let args = command.get_args().collect::<Vec<_>>();
                let output_index = args
                    .iter()
                    .position(|arg| *arg == OsStr::new("--output-last-message"))
                    .expect("output flag");
                if let Err(error) = fs::write(Path::new(args[output_index + 1]), "partial") {
                    return std::future::ready(Err(error.into()));
                }
                return std::future::ready(Err(anyhow::anyhow!("interrupted")));
            }
            std::future::ready(Ok(true))
        },
    )
    .await
    .expect_err("research should fail");

    assert_eq!(calls, 2);
    assert_eq!(error.to_string(), "interrupted");
    assert_eq!(fs::read_dir(&report_dir)?.count(), 0);
    assert!(!isolated_home.expect("isolated home captured").exists());
    Ok(())
}
