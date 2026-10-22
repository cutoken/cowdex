//! Offline usage reports from retained rollout records, without model requests.

use std::path::PathBuf;

use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use clap::Args;

mod chart;
mod history;

#[derive(Debug, Args)]
pub(crate) struct Options {
    /// Export a project token chart for this interval (e.g. 24h, 7d, 1month).
    #[arg(long, num_args = 0..=1, default_missing_value = "24h", value_name = "INTERVAL", value_parser = parse_interval, conflicts_with = "prompt")]
    pub export_usage: Option<Interval>,

    /// Where to save the usage PNG (default: cowdex-usage-<interval>.png).
    #[arg(long = "usage-output", value_name = "PNG", requires = "export_usage")]
    output: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Interval {
    label: String,
    seconds: i64,
}

fn parse_interval(value: &str) -> Result<Interval, String> {
    let value = value.to_ascii_lowercase();
    let split = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    let (amount, unit) = value.split_at(split);
    let amount: i64 = amount
        .parse()
        .map_err(|_| "Use an interval such as 24h, 7d or 1month")?;
    let multiplier = match unit {
        "h" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        "w" | "week" | "weeks" => 7 * 86_400,
        "mo" | "month" | "months" => 30 * 86_400,
        _ => return Err("Use h, d, w or month (a month is 30 days)".into()),
    };
    let seconds = amount
        .checked_mul(multiplier)
        .filter(|n| *n > 0 && Duration::try_seconds(*n).is_some())
        .ok_or("Interval must be positive and fit in a duration")?;
    Ok(Interval {
        label: value,
        seconds,
    })
}

pub(crate) fn run(options: Options, end: DateTime<Utc>) -> anyhow::Result<()> {
    let interval = options
        .export_usage
        .ok_or_else(|| anyhow::anyhow!("Missing usage interval"))?;
    let start = end
        .checked_sub_signed(Duration::seconds(interval.seconds))
        .ok_or_else(|| anyhow::anyhow!("Interval exceeds the supported date range"))?;
    let report = history::collect(codex_core::config::find_codex_home()?.as_path(), start, end)?;
    let output = options
        .output
        .unwrap_or_else(|| PathBuf::from(format!("cowdex-usage-{}.png", interval.label)));
    anyhow::ensure!(
        output.extension().is_some_and(|ext| ext
            .to_str()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))),
        "--usage-output must name a .png file"
    );
    chart::write_png(&report, start, end, &output)?;
    println!(
        "Saved {} ({} projects, {} tokens).",
        output.display(),
        report.projects.len(),
        report.total().total()
    );
    if report.skipped > 0 {
        eprintln!(
            "Skipped {} unreadable or malformed history records.",
            report.skipped
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "usage_export_tests.rs"]
mod tests;
