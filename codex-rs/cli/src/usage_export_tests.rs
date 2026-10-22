use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::Utc;
use clap::Parser;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;

use super::chart;
use super::history;
use super::history::Report;
use super::history::Tokens;
use crate::MultitoolCli;

fn window() -> (DateTime<Utc>, DateTime<Utc>) {
    (
        "2026-10-21T12:00:00Z".parse().expect("valid test fixture"),
        "2026-10-22T12:00:00Z".parse().expect("valid test fixture"),
    )
}

fn usage(input: i64, cached: i64, output: i64, reasoning: i64) -> Value {
    json!({"input_tokens":input,"cached_input_tokens":cached,"output_tokens":output,"reasoning_output_tokens":reasoning,"total_tokens":input+output})
}

fn meta(id: &str, cwd: &str) -> Value {
    json!({"type":"session_meta","payload":{"id":id,"cwd":cwd}})
}

fn event(time: &str, payload: Value) -> Value {
    json!({"timestamp":time,"type":"event_msg","payload":payload})
}

fn raw(id: &str, tokens: Value) -> Value {
    event(
        "2026-10-22T10:00:00Z",
        json!({"type":"raw_response_completed","response_id":id,"token_usage":tokens}),
    )
}

fn count(time: &str, total: Value, last: Value) -> Value {
    event(
        time,
        json!({"type":"token_count","info":{"total_token_usage":total,"last_token_usage":last}}),
    )
}

fn rollout(home: &Path, name: &str, records: Vec<Value>) {
    let path = home.join(name);
    fs::create_dir_all(path.parent().expect("valid test fixture")).expect("valid test fixture");
    let body = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(path, body).expect("valid test fixture");
}

#[test]
fn export_cli_defaults_and_validates_interval_and_output() {
    let default =
        MultitoolCli::try_parse_from(["cowdex", "--export-usage"]).expect("valid test fixture");
    assert_eq!(
        default.usage.export_usage,
        Some(super::parse_interval("24h").expect("valid test fixture"))
    );
    for interval in ["1month", "7d", "2w", "48h"] {
        let cli = MultitoolCli::try_parse_from([
            "cowdex",
            "--export-usage",
            interval,
            "--usage-output",
            "report.png",
        ])
        .expect("valid test fixture");
        assert_eq!(cli.usage.output, Some(PathBuf::from("report.png")));
    }
    for interval in ["0h", "-1d", "1year", "forever", "99999999999999999month"] {
        assert!(MultitoolCli::try_parse_from(["cowdex", "--export-usage", interval]).is_err());
    }
    assert!(MultitoolCli::try_parse_from(["cowdex", "--usage-output", "report.png"]).is_err());
}

#[test]
fn raw_usage_deduplicates_forks_and_cumulative_notifications() {
    let home = tempfile::tempdir().expect("valid test fixture");
    let tokens = usage(
        /*input*/ 100, /*cached*/ 70, /*output*/ 20, /*reasoning*/ 5,
    );
    rollout(
        home.path(),
        "sessions/rollout-parent.jsonl",
        vec![
            meta("main", "/work/alpha"),
            raw("response-1", tokens.clone()),
            count("2026-10-22T10:00:01Z", tokens.clone(), tokens.clone()),
            count("2026-10-22T10:00:02Z", tokens.clone(), tokens.clone()),
        ],
    );
    rollout(
        home.path(),
        "archived_sessions/rollout-worker.jsonl",
        vec![
            meta("child", "/work/alpha"),
            raw("response-1", tokens),
            raw(
                "response-2",
                usage(
                    /*input*/ 50, /*cached*/ 10, /*output*/ 10, /*reasoning*/ 4,
                ),
            ),
        ],
    );
    let (start, end) = window();
    let report = history::collect(home.path(), start, end).expect("valid test fixture");
    assert_eq!(
        report.projects,
        BTreeMap::from([(PathBuf::from("/work/alpha"), Tokens([80, 70, 9, 21]))])
    );
    assert_eq!(report.skipped, 0);
}

#[test]
fn legacy_usage_uses_deltas_and_window_boundaries() {
    let home = tempfile::tempdir().expect("valid test fixture");
    rollout(
        home.path(),
        "sessions/rollout-legacy.jsonl",
        vec![
            meta("old", "/work/old"),
            count(
                "2026-10-21T11:59:59Z",
                usage(
                    /*input*/ 100, /*cached*/ 50, /*output*/ 10,
                    /*reasoning*/ 2,
                ),
                usage(
                    /*input*/ 100, /*cached*/ 50, /*output*/ 10,
                    /*reasoning*/ 2,
                ),
            ),
            count(
                "2026-10-21T12:00:00Z",
                usage(
                    /*input*/ 200, /*cached*/ 120, /*output*/ 30,
                    /*reasoning*/ 7,
                ),
                usage(
                    /*input*/ 100, /*cached*/ 70, /*output*/ 20,
                    /*reasoning*/ 5,
                ),
            ),
            count(
                "2026-10-22T11:00:00Z",
                usage(
                    /*input*/ 200, /*cached*/ 120, /*output*/ 30,
                    /*reasoning*/ 7,
                ),
                usage(
                    /*input*/ 100, /*cached*/ 70, /*output*/ 20,
                    /*reasoning*/ 5,
                ),
            ),
            count(
                "2026-10-22T12:00:01Z",
                usage(
                    /*input*/ 300, /*cached*/ 190, /*output*/ 50,
                    /*reasoning*/ 12,
                ),
                usage(
                    /*input*/ 100, /*cached*/ 70, /*output*/ 20,
                    /*reasoning*/ 5,
                ),
            ),
        ],
    );
    let (start, end) = window();
    assert_eq!(
        history::collect(home.path(), start, end)
            .expect("valid test fixture")
            .projects,
        BTreeMap::from([(PathBuf::from("/work/old"), Tokens([70, 30, 5, 15]))])
    );
}

#[test]
fn compressed_backups_and_trimmed_rollouts_count_once() {
    let home = tempfile::tempdir().expect("valid test fixture");
    let records = vec![
        meta("main", "/work/alpha"),
        raw(
            "same-response",
            usage(
                /*input*/ 80, /*cached*/ 60, /*output*/ 20, /*reasoning*/ 12,
            ),
        ),
    ];
    rollout(home.path(), "sessions/rollout-main.jsonl", records.clone());
    let backup = home.path().join("backups/rollouts");
    fs::create_dir_all(&backup).expect("valid test fixture");
    let body = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(
        backup.join("rollout-main.jsonl-original.zst"),
        zstd::stream::encode_all(body.as_bytes(), /*level*/ 1).expect("valid test fixture"),
    )
    .expect("valid test fixture");
    let (start, end) = window();
    assert_eq!(
        history::collect(home.path(), start, end)
            .expect("valid test fixture")
            .total(),
        Tokens([60, 20, 12, 8])
    );
}

#[test]
fn inherited_ordinals_and_large_tool_output_are_ignored() {
    let home = tempfile::tempdir().expect("valid test fixture");
    let mut metadata = meta("child", "/work/child");
    metadata["payload"]["subagent_history_start_ordinal"] = json!(3);
    let mut inherited = raw(
        "inherited",
        usage(
            /*input*/ 1000, /*cached*/ 900, /*output*/ 100, /*reasoning*/ 90,
        ),
    );
    inherited["ordinal"] = json!(1);
    let mut own = raw(
        "own",
        usage(
            /*input*/ 100, /*cached*/ 80, /*output*/ 10, /*reasoning*/ 6,
        ),
    );
    own["ordinal"] = json!(4);
    rollout(
        home.path(),
        "sessions/rollout-child.jsonl",
        vec![
            metadata,
            inherited,
            json!({"type":"response_item","payload":"x".repeat(2*1024*1024)}),
            own,
        ],
    );
    let (start, end) = window();
    assert_eq!(
        history::collect(home.path(), start, end)
            .expect("valid test fixture")
            .total(),
        Tokens([80, 20, 6, 4])
    );
}

#[test]
fn chart_snapshot_and_png_are_deterministic() {
    let report = Report {
        projects: BTreeMap::from([
            (
                PathBuf::from("/work/Alpha & Beta"),
                Tokens([7_500_000, 1_500_000, 300_000, 700_000]),
            ),
            (
                PathBuf::from("/work/notes"),
                Tokens([500_000, 300_000, 100_000, 100_000]),
            ),
        ]),
        ..Default::default()
    };
    let (start, end) = window();
    insta::assert_snapshot!(
        "usage_chart",
        chart::svg(&report, start, end).expect("valid test fixture")
    );
    let temp = tempfile::tempdir().expect("valid test fixture");
    let first = temp.path().join("first.png");
    let second = temp.path().join("second.png");
    chart::write_png(&report, start, end, &first).expect("valid test fixture");
    chart::write_png(&report, start, end, &second).expect("valid test fixture");
    let bytes = fs::read(first).expect("valid test fixture");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(bytes, fs::read(second).expect("valid test fixture"));
}

#[test]
fn empty_report_still_produces_a_readable_chart() {
    let (start, end) = window();
    insta::assert_snapshot!(
        "usage_chart_empty",
        chart::svg(&Report::default(), start, end).expect("valid test fixture")
    );
}

#[test]
fn trimmed_legacy_checkpoint_does_not_repeat_usage_from_backup() {
    let home = tempfile::tempdir().expect("test directory");
    let checkpoint = count(
        "2026-10-22T10:00:00Z",
        usage(
            /*input*/ 200, /*cached*/ 100, /*output*/ 20, /*reasoning*/ 5,
        ),
        usage(
            /*input*/ 100, /*cached*/ 50, /*output*/ 10, /*reasoning*/ 2,
        ),
    );
    let original = vec![
        meta("main", "/work/alpha"),
        event(
            "2026-10-22T09:00:00Z",
            json!({"type":"turn_started","turn_id":"turn-1"}),
        ),
        checkpoint.clone(),
    ];
    rollout(home.path(), "backups/rollouts/rollout-main.jsonl", original);
    rollout(
        home.path(),
        "sessions/rollout-main.jsonl",
        vec![meta("main", "/work/alpha"), checkpoint],
    );
    let (start, end) = window();
    assert_eq!(
        history::collect(home.path(), start, end)
            .expect("usage report")
            .total(),
        Tokens([50, 50, 2, 8])
    );
}
