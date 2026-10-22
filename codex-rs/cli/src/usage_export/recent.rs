//! Seek to a recent JSONL suffix without parsing old conversation payloads.

use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;

use chrono::DateTime;
use chrono::Utc;

#[derive(Clone, Copy)]
pub(super) enum Scan {
    Recent,
    Full,
}

pub(super) fn open(
    path: &Path,
    start: DateTime<Utc>,
    scan: Scan,
) -> std::io::Result<Box<dyn BufRead>> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    if path.extension().is_some_and(|ext| ext == "zst") {
        return Ok(Box::new(BufReader::new(zstd::stream::read::Decoder::new(
            file.take(length),
        )?)));
    }
    let offset = match scan {
        Scan::Recent => start_offset(&mut file, start)?,
        Scan::Full => 0,
    };
    file.seek(SeekFrom::Start(offset))?;
    Ok(Box::new(BufReader::new(file.take(length - offset))))
}

pub(super) fn start_offset(file: &mut File, start: DateTime<Utc>) -> std::io::Result<u64> {
    let mut reader = BufReader::new(file);
    let mut low = 0;
    let mut high = reader.get_ref().metadata()?.len();
    let mut prefix = Vec::new();
    // Rollout timestamps are written in order. Unknown formats fall back to a full scan.
    while high.saturating_sub(low) > 128 * 1024 {
        let middle = low + (high - low) / 2;
        reader.seek(SeekFrom::Start(middle))?;
        reader.skip_until(b'\n')?;
        prefix.clear();
        Read::by_ref(&mut reader)
            .take(512)
            .read_until(b'\n', &mut prefix)?;
        if prefix.is_empty() {
            high = middle;
            continue;
        }
        let Some(timestamp) = timestamp(&prefix) else {
            return Ok(0);
        };
        if timestamp < start {
            if !prefix.ends_with(b"\n") {
                reader.skip_until(b'\n')?;
            }
            low = reader.stream_position()?;
        } else {
            high = middle;
        }
    }
    // Include a little preceding history to seed legacy cumulative-counter deltas.
    let offset = low.saturating_sub(1024 * 1024);
    if offset == 0 {
        return Ok(0);
    }
    reader.seek(SeekFrom::Start(offset))?;
    reader.skip_until(b'\n')?;
    reader.stream_position()
}

fn timestamp(prefix: &[u8]) -> Option<DateTime<Utc>> {
    let text = String::from_utf8_lossy(prefix);
    let (_, value) = text.split_once("\"timestamp\"")?;
    let value = value
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?
        .split('"')
        .next()?;
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}
