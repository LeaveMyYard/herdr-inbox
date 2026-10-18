//! Did a delivery land in a transcript? (Python `transcript.landed_kinds`.)
//!
//! A delivery counts only once the transcript shows it, judged by its FULL
//! expected text, never by the id alone: a record counts for an id when it
//! names the id where the inbox writes its header, holds one of the
//! delivery's exact strings at a segment start, and is not older than the
//! answer (ids restart when the database is recreated).

mod records;
mod tails;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use herdr_agent_kit::agent_session::claude::wire::ts;
use herdr_agent_kit::internal::inbox_contract::delivery::header_ids;
use serde_json::Value;

pub use records::{Texts, record_texts};
pub use tails::{strip_paste_open, tails};

use crate::text::squash;

/// Never read more than this from a transcript's tail (Python `MAX_READ`).
pub const MAX_READ: u64 = 8 * 1024 * 1024;

/// What one question's delivery carries: the exact strings (single message,
/// batch segment, long-answer reference) and when it was answered.
#[derive(Clone, Debug)]
pub struct Expected {
    /// The strings, as typed (see `delivery::expected_segments`).
    pub segments: Vec<String>,
    /// The answer's time (`answered_at`): an older record never counts.
    pub since: Option<String>,
}

/// Which of `expected`'s ids landed in `path`, each with the indexes of the
/// strings that matched. Empty when the file cannot be read.
pub fn landed_kinds(
    path: &Path,
    expected: &BTreeMap<u64, Expected>,
) -> BTreeMap<u64, BTreeSet<usize>> {
    let mut found: BTreeMap<u64, BTreeSet<usize>> = BTreeMap::new();
    let Some(data) = tail(path) else {
        return found;
    };
    let squashed: BTreeMap<u64, Vec<String>> = expected
        .iter()
        .map(|(id, e)| (*id, e.segments.iter().map(|s| squash(s)).collect()))
        .collect();
    for raw in data.split(|&b| b == b'\n') {
        if !raw.windows(7).any(|w| w == b"[inbox]") {
            continue;
        }
        let Texts { texts, ts: when } = record_texts(&String::from_utf8_lossy(raw));
        for text in &texts {
            for tail in tails(text) {
                let named = header_ids(tail);
                let stripped = squash(tail);
                for (id, e) in expected {
                    if !named.contains(id) || !after(when.as_deref(), e.since.as_deref()) {
                        continue;
                    }
                    for (i, seg) in squashed[id].iter().enumerate() {
                        if stripped.starts_with(seg.as_str())
                            || stripped.contains(&format!("— {seg}"))
                            || stripped.contains(&format!(" · {seg}"))
                        {
                            found.entry(*id).or_default().insert(i);
                        }
                    }
                }
            }
        }
    }
    found
}

/// The record is not older than `since` (Python `_after`): lenient, a missing
/// or unreadable time on either side counts as newer. Both are read with the
/// kit's timestamp reader (Python used `datetime.fromisoformat`; they agree on
/// every ISO-8601 time Claude Code and the inbox write).
fn after(ts_text: Option<&str>, since: Option<&str>) -> bool {
    let (Some(at), Some(since)) = (
        ts_text.filter(|s| !s.is_empty()),
        since.filter(|s| !s.is_empty()),
    ) else {
        return true;
    };
    match (iso_ts(at), iso_ts(since)) {
        (Some(a), Some(b)) => a >= b,
        _ => true,
    }
}

/// An ISO-8601 time in epoch seconds. A bare number is not one: Python's
/// `datetime.fromisoformat` raises on it (`_after` then counts the record
/// as newer), while the kit's `ts` would read it as epoch seconds.
fn iso_ts(text: &str) -> Option<f64> {
    if text
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'+')
        && !text.contains(':')
    {
        return None;
    }
    ts(&Value::String(text.to_owned())).map(|t| t.secs())
}

/// The last [`MAX_READ`] bytes of the file.
fn tail(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let size = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(size.saturating_sub(MAX_READ)))
        .ok()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    Some(data)
}

#[cfg(test)]
mod tests;
