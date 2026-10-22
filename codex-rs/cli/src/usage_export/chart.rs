use std::fmt::Write;
use std::path::Path;

use anyhow::Context;
use chrono::DateTime;
use chrono::Utc;
use resvg::tiny_skia;
use resvg::usvg;

use super::history::Report;

const COLORS: [&str; 4] = ["#277d71", "#769cc5", "#9d7fbb", "#dd9d58"];

pub(super) fn write_png(
    report: &Report,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    path: &Path,
) -> anyhow::Result<()> {
    let svg = svg(report, start, end)?;
    let mut options = usvg::Options {
        font_family: "DM Sans".into(),
        ..Default::default()
    };
    options
        .fontdb_mut()
        .load_font_data(include_bytes!("../../assets/fonts/DMSans.ttf").to_vec());
    let tree = usvg::Tree::from_str(&svg, &options)?;
    let size = tree.size().to_int_size();
    let mut pixels = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| anyhow::anyhow!("Not enough memory to draw the usage chart"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut output = tempfile::NamedTempFile::new_in(parent).context("Cannot create usage PNG")?;
    std::io::Write::write_all(&mut output, &pixels.encode_png()?)?;
    output.persist(path).context("Cannot save usage PNG")?;
    Ok(())
}

pub(super) fn svg(
    report: &Report,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> anyhow::Result<String> {
    let rows = report.projects.len().max(1);
    anyhow::ensure!(
        rows <= 400,
        "Too many projects for one PNG; choose a shorter interval (maximum 400 projects)"
    );
    let height = 390 + rows * 70;
    let total = report.total();
    let cache_share = if total.total() > 0 {
        100.0 * total.0[0] as f64 / total.0[0].saturating_add(total.0[1]).max(1) as f64
    } else {
        0.0
    };
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="{height}" viewBox="0 0 1600 {height}">
<defs>
  <linearGradient id="paper" x2="0.8" y2="1"><stop stop-color="#f8f5ee"/><stop offset="1" stop-color="#e9ede9"/></linearGradient>
  <linearGradient id="ink" x2="1" y2="1"><stop stop-color="#243c3a"/><stop offset="1" stop-color="#172a2b"/></linearGradient>
  <linearGradient id="sheen" x2="0" y2="1"><stop stop-color="#fff" stop-opacity=".18"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity=".1"/></linearGradient>
  <filter id="shadow" x="-10%" y="-10%" width="120%" height="130%"><feDropShadow dx="0" dy="5" stdDeviation="8" flood-color="#233d36" flood-opacity=".1"/></filter>
</defs>
<rect width="1600" height="{height}" fill="url(#paper)"/>
<g font-family="DM Sans" fill="#243c3a">
<text x="64" y="58" font-size="19" letter-spacing="3">COWDEX / TOKEN USAGE</text>
<text x="64" y="119" font-size="42">Token usage by project</text>
<text x="64" y="154" font-size="19" fill="#61736c">{} — {} UTC</text>
<rect x="1030" y="42" width="506" height="122" rx="18" fill="url(#ink)" filter="url(#shadow)"/>
<text x="1058" y="87" font-size="33" fill="#fff">{} tokens</text>
<text x="1058" y="132" font-size="18" fill="#b8d2c9">{} projects · {:.0}% of input cached</text>
<rect x="40" y="198" width="1520" height="{}" rx="20" fill="#fffefa" filter="url(#shadow)"/>
"##,
        start.format("%d %b %Y, %H:%M"),
        end.format("%d %b %Y, %H:%M"),
        number(total.total()),
        report.projects.len(),
        cache_share,
        height - 274
    );
    for (i, label) in [
        "Cached input",
        "Uncached input",
        "Reasoning",
        "Other output",
    ]
    .iter()
    .enumerate()
    {
        let x = 70 + i * 330;
        write!(
            svg,
            r#"<rect x="{x}" y="228" width="12" height="12" rx="3" fill="{}"/><text x="{}" y="241" font-size="18">{label}</text>"#,
            COLORS[i],
            x + 23
        )?;
    }
    let mut projects: Vec<_> = report.projects.iter().collect();
    projects.sort_by(|(left_path, left), (right_path, right)| {
        right
            .total()
            .cmp(&left.total())
            .then_with(|| left_path.cmp(right_path))
    });
    let max = projects
        .iter()
        .map(|(_, tokens)| tokens.total())
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let bar_left = 400.0;
    let bar_width = 920.0;
    for i in 0..=4 {
        let x = bar_left + bar_width * i as f64 / 4.0;
        write!(
            svg,
            r##"<path d="M{x} 286 V{}" stroke="#e5e9e3" stroke-dasharray="3 6"/><text x="{x}" y="277" font-size="15" text-anchor="middle" fill="#78867f">{}</text>"##,
            height - 112,
            number((max * i as f64 / 4.0) as u64)
        )?;
    }
    for (row, (path, tokens)) in projects.into_iter().enumerate() {
        let y = 309 + row * 70;
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let duplicates = report
            .projects
            .keys()
            .filter(|other| other.file_name() == path.file_name())
            .count()
            > 1;
        let label = if duplicates {
            path.components()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        } else {
            name.into_owned()
        };
        let label = if label.chars().count() > 29 {
            format!("{}…", label.chars().take(28).collect::<String>())
        } else {
            label
        };
        write!(
            svg,
            r##"<text x="70" y="{}" font-size="21">{}</text><text x="1518" y="{}" text-anchor="end" font-size="20">{}</text><clipPath id="bar{row}"><rect x="400" y="{y}" width="{}" height="30" rx="6"/></clipPath><g clip-path="url(#bar{row})">"##,
            y + 22,
            escape(&label),
            y + 22,
            number(tokens.total()),
            bar_width * tokens.total() as f64 / max
        )?;
        let mut x = bar_left;
        for (value, color) in tokens.0.iter().zip(COLORS) {
            let width = bar_width * *value as f64 / max;
            write!(
                svg,
                r#"<rect x="{x}" y="{y}" width="{width}" height="30" fill="{color}"/>"#
            )?;
            x += width;
        }
        write!(
            svg,
            r#"<rect x="400" y="{y}" width="{}" height="30" fill="url(#sheen)"/></g>"#,
            x - bar_left
        )?;
    }
    if report.projects.is_empty() {
        svg.push_str(r##"<text x="70" y="331" font-size="22" fill="#61736c">No recorded usage in this interval.</text>"##);
    }
    write!(
        svg,
        r##"<text x="64" y="{}" font-size="16" fill="#61736c">Main agents + workers · Retained local history · Tokens, not cost</text></g></svg>"##,
        height - 34
    )?;
    Ok(svg)
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

fn number(value: u64) -> String {
    match value {
        1_000_000_000.. => format!("{:.2}B", value as f64 / 1_000_000_000.0),
        1_000_000.. => format!("{:.2}M", value as f64 / 1_000_000.0),
        1_000.. => format!("{:.1}K", value as f64 / 1_000.0),
        _ => value.to_string(),
    }
}
