use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::io::BufRead;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use chrono::DateTime;
use chrono::Utc;
use codex_protocol::protocol::TokenUsage;
use serde_json::Value;

use super::progress::Progress;
use super::recent;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Tokens(pub [u64; 4]);

impl Tokens {
    fn from_usage(usage: &TokenUsage) -> Self {
        let input = usage.input_tokens.max(0) as u64;
        let cached = (usage.cached_input_tokens.max(0) as u64).min(input);
        let output = usage.output_tokens.max(0) as u64;
        let reasoning = (usage.reasoning_output_tokens.max(0) as u64).min(output);
        Self([cached, input - cached, reasoning, output - reasoning])
    }

    pub(super) fn total(&self) -> u64 {
        self.0.iter().copied().fold(0, u64::saturating_add)
    }

    fn add(&mut self, other: &Self) {
        for (current, next) in self.0.iter_mut().zip(other.0) {
            *current = current.saturating_add(next);
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Report {
    pub projects: BTreeMap<PathBuf, Tokens>,
    pub skipped: usize,
}

impl Report {
    pub(super) fn total(&self) -> Tokens {
        self.projects
            .values()
            .fold(Tokens::default(), |mut total, tokens| {
                total.add(tokens);
                total
            })
    }
}

pub(super) fn collect(
    home: &Path,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> anyhow::Result<Report> {
    eprintln!("Finding recent usage files...");
    let mut paths = Vec::new();
    for dir in ["backups/rollouts", "sessions", "archived_sessions"] {
        files(&home.join(dir), &mut paths)?;
    }
    paths.sort_by_cached_key(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        (
            name.split(".jsonl").next().unwrap_or_default().to_owned(),
            !path.starts_with(home.join("backups")),
        )
    });
    let recent_paths: Vec<_> = paths
        .iter()
        .filter(|path| {
            fs::metadata(path)
                .and_then(|meta| meta.modified())
                .map(DateTime::<Utc>::from)
                .map_or(true, |modified| modified >= start)
        })
        .cloned()
        .collect();
    // Unmarked legacy fork copies need ancestor response IDs to avoid double billing.
    let full = recent_paths.iter().any(|path| {
        header(path, start).is_ok_and(|record| {
            let meta = &record["payload"];
            !meta["forked_from_id"].is_null()
                && meta["history_base"].is_null()
                && meta["subagent_history_start_ordinal"].is_null()
        })
    });
    let (paths, mode) = if full {
        (paths, recent::Scan::Full)
    } else {
        (recent_paths, recent::Scan::Recent)
    };
    let mut report = Report::default();
    let mut seen = HashSet::new();
    let mut progress = Progress::new(paths.len());
    for (index, path) in paths.iter().enumerate() {
        progress.begin(index + 1);
        let result = scan(
            path,
            start,
            end,
            &mut seen,
            &mut report,
            mode,
            &mut progress,
        );
        if result.is_err() {
            report.skipped += 1;
        }
    }
    progress.finish();
    Ok(report)
}

fn files(dir: &Path, paths: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("Cannot read {}", dir.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            files(&entry.path(), paths)?;
        } else if kind.is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("rollout-") && (name.ends_with(".jsonl") || name.ends_with(".zst"))
            {
                // During compression both representations can briefly exist.
                if !name.ends_with(".zst") || !entry.path().with_extension("").exists() {
                    paths.push(entry.path());
                }
            }
        }
    }
    Ok(())
}

fn scan(
    path: &Path,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    seen: &mut HashSet<String>,
    report: &mut Report,
    mode: recent::Scan,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    let metadata = header(path, start)?;
    let metadata = &metadata["payload"];
    let mut reader = recent::open(path, start, mode)?;
    let mut buffer = Vec::new();
    let mut project = metadata["cwd"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("Unknown project"));
    let mut previous: Option<TokenUsage> = None;
    let mut own_start = metadata["subagent_history_start_ordinal"]
        .as_u64()
        .unwrap_or(0);
    let mut thread = metadata["id"].as_str().unwrap_or_default().to_owned();
    let mut turn = String::new();
    let mut raw_since_count = false;
    while read_record(&mut reader, &mut buffer)? {
        progress.record();
        // Avoid deserializing tool output, prompts and other large history payloads.
        if ![
            b"session_meta".as_slice(),
            b"raw_response_completed",
            b"token_count",
            b"turn_started",
            b"task_started",
        ]
        .iter()
        .any(|needle| buffer.windows(needle.len()).any(|part| part == *needle))
        {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<Value>(&buffer) else {
            report.skipped += 1;
            continue;
        };
        let payload = &record["payload"];
        if record["type"] == "session_meta" {
            if let Some(cwd) = payload["cwd"].as_str() {
                project = PathBuf::from(cwd);
            }
            thread = payload["id"].as_str().unwrap_or_default().to_owned();
            own_start = payload["subagent_history_start_ordinal"]
                .as_u64()
                .unwrap_or(0);
            continue;
        }
        if record["type"] != "event_msg"
            || record["ordinal"]
                .as_u64()
                .is_some_and(|ordinal| ordinal < own_start)
        {
            continue;
        }
        let kind = payload["type"].as_str().unwrap_or_default();
        if matches!(kind, "turn_started" | "task_started") {
            turn = payload["turn_id"].as_str().unwrap_or_default().to_owned();
            continue;
        }
        let timestamp = record["timestamp"]
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let in_range = timestamp.is_some_and(|time| time >= start && time <= end);
        let (usage, key) = if kind == "raw_response_completed" {
            let Ok(usage) = serde_json::from_value::<TokenUsage>(payload["token_usage"].clone())
            else {
                continue;
            };
            raw_since_count = true;
            let id = payload["response_id"].as_str().filter(|id| !id.is_empty());
            let key = id.map_or_else(
                || format!("raw:{thread}:{}:{}", record["timestamp"], record["ordinal"]),
                |id| format!("raw:{id}"),
            );
            (usage, key)
        } else if kind == "token_count" {
            let info = &payload["info"];
            let Ok(total) = serde_json::from_value::<TokenUsage>(info["total_token_usage"].clone())
            else {
                continue;
            };
            let usage = if let Some(prev) = &previous
                && total.input_tokens >= prev.input_tokens
                && total.output_tokens >= prev.output_tokens
            {
                TokenUsage {
                    input_tokens: total.input_tokens.saturating_sub(prev.input_tokens).max(0),
                    cached_input_tokens: total
                        .cached_input_tokens
                        .saturating_sub(prev.cached_input_tokens)
                        .max(0),
                    output_tokens: total
                        .output_tokens
                        .saturating_sub(prev.output_tokens)
                        .max(0),
                    reasoning_output_tokens: total
                        .reasoning_output_tokens
                        .saturating_sub(prev.reasoning_output_tokens)
                        .max(0),
                    ..Default::default()
                }
            } else {
                serde_json::from_value(info["last_token_usage"].clone()).unwrap_or_default()
            };
            let key = format!(
                "legacy:{thread}:{}:{:?}",
                record["timestamp"],
                Tokens::from_usage(&total)
            );
            let turn_key =
                (!turn.is_empty()).then(|| format!("turn:{turn}:{:?}", Tokens::from_usage(&total)));
            previous = Some(total);
            if std::mem::take(&mut raw_since_count) {
                seen.insert(key);
                if let Some(turn_key) = turn_key {
                    seen.insert(turn_key);
                }
                continue;
            }
            if turn_key.is_some_and(|key| !seen.insert(key)) {
                continue;
            }
            (usage, key)
        } else {
            continue;
        };
        if seen.insert(key) && in_range {
            let tokens = Tokens::from_usage(&usage);
            if tokens.total() > 0 {
                report
                    .projects
                    .entry(project.clone())
                    .or_default()
                    .add(&tokens);
            }
        }
    }
    Ok(())
}

fn header(path: &Path, start: DateTime<Utc>) -> anyhow::Result<Value> {
    let mut reader = recent::open(path, start, recent::Scan::Full)?;
    let mut record = Vec::new();
    read_record(&mut reader, &mut record)?;
    Ok(serde_json::from_slice(&record)?)
}

// Bound memory even if a rollout contains a multi-gigabyte tool-output line.
fn read_record(reader: &mut dyn BufRead, output: &mut Vec<u8>) -> std::io::Result<bool> {
    output.clear();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(!output.is_empty());
        }
        let end = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|n| n + 1);
        let count = end.unwrap_or(available.len());
        if output.len() + count > 1024 * 1024 {
            oversized = true;
            output.clear();
        }
        if !oversized {
            output.extend_from_slice(&available[..count]);
        }
        reader.consume(count);
        if end.is_some() {
            return Ok(true);
        }
    }
}
