use anyhow::Result;
use chrono::Utc;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn usage_export_runs_offline_with_default_and_explicit_intervals() -> Result<()> {
    let home = tempfile::tempdir()?;
    let sessions = home.path().join("sessions");
    std::fs::create_dir(&sessions)?;
    let records = [
        json!({"type":"session_meta","payload":{"id":"offline","cwd":"/work/example"}}),
        json!({"type":"event_msg","timestamp":Utc::now().to_rfc3339(),"payload":{"type":"raw_response_completed","response_id":"offline-response","token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":20,"reasoning_output_tokens":5,"total_tokens":120}}}),
    ];
    std::fs::write(
        sessions.join("rollout-offline.jsonl"),
        records
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )?;
    // Deliberately unusable model configuration proves export does not load a model.
    std::fs::write(
        home.path().join("config.toml"),
        "model_provider = 'not-a-provider'\n",
    )?;
    let binary = codex_utils_cargo_bin::cargo_bin("cowdex")?;
    let mut command = assert_cmd::Command::new(&binary);
    command
        .env("CODEX_HOME", home.path())
        .current_dir(home.path())
        .arg("--export-usage");
    let result = command.assert().success().get_output().clone();
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 projects, 120 tokens"));
    let stderr = String::from_utf8_lossy(&result.stderr);
    let progress: Vec<_> = stderr
        .lines()
        .filter(|line| {
            line.starts_with("Finding recent")
                || line.starts_with("Reading usage: 0/")
                || line.starts_with("Reading usage: 1/1 files (2 records)")
                || *line == "Rendering PNG..."
        })
        .collect();
    insta::assert_snapshot!("usage_export_progress", progress.join("\n"));
    let default = std::fs::read(home.path().join("cowdex-usage-24h.png"))?;
    assert_eq!(&default[..8], b"\x89PNG\r\n\x1a\n");
    let output = home.path().join("month.png");
    assert_cmd::Command::new(&binary)
        .env("CODEX_HOME", home.path())
        .args(["--export-usage", "1month", "--usage-output"])
        .arg(&output)
        .assert()
        .success();
    assert!(output.is_file());
    assert_cmd::Command::new(binary)
        .env("CODEX_HOME", home.path())
        .args(["--export-usage", "--usage-output"])
        .arg(home.path().join("missing/report.png"))
        .assert()
        .failure()
        .stderr(predicates::str::contains("Cannot create usage PNG"));
    Ok(())
}
