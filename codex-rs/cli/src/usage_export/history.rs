use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use chrono::DateTime;
use chrono::Utc;
use codex_protocol::protocol::TokenUsage;
use serde_json::Value;

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
    let mut report = Report::default();
    let mut seen = HashSet::new();
    for path in paths {
        let result = scan(&path, start, end, &mut seen, &mut report);
        if result.is_err() {
            report.skipped += 1;
        }
    }
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
) -> anyhow::Result<()> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    let file = file.take(length);
    let mut reader: Box<dyn BufRead> = if path.extension().is_some_and(|ext| ext == "zst") {
        Box::new(BufReader::new(zstd::stream::read::Decoder::new(file)?))
    } else {
        Box::new(BufReader::new(file))
    };
    let mut buffer = Vec::new();
    let mut project = PathBuf::from("Unknown project");
    let mut previous: Option<TokenUsage> = None;
    let mut own_start = 0;
    let mut thread = String::new();
    let mut turn = String::new();
    let mut raw_since_count = false;
    while read_record(&mut reader, &mut buffer)? {
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
