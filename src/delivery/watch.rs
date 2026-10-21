//! Waiting for a typed message to show in a transcript (Python
//! `transcript.contains`, `landed_since`, `wait_landed_any`, `size` and
//! `service._wait_hits`).

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use herdr_agent_kit::internal::inbox_contract::delivery::header_ids;

use crate::claude::landed::{MAX_READ, record_texts, tails};
use crate::text::squash;

/// How often a wait looks again (Python `interval`).
pub const INTERVAL: Duration = Duration::from_millis(250);

/// A transcript watched from an offset: where a typed message may land.
pub type Watched = (PathBuf, u64);

/// The file's size, 0 when it cannot be read (Python `transcript.size`).
pub fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

/// Our message for `qids` in `text` (Python `transcript.contains`): at the
/// record's start naming every id, or later in it only when the whole typed
/// message `full` follows there (whitespace folded), never a mere mention.
pub fn contains(text: &str, qids: &[u64], full: Option<&str>) -> bool {
    let names_all = |tail: &str| {
        let named = header_ids(tail);
        qids.iter().all(|q| named.contains(q))
    };
    tails(text).into_iter().enumerate().any(|(i, tail)| {
        if i == 0 {
            names_all(tail)
        } else {
            full.is_some_and(|f| squash(tail).starts_with(&squash(f)) && names_all(tail))
        }
    })
}

/// The bytes after `offset` (at most the last [`MAX_READ`]).
fn since(path: &Path, offset: u64) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let end = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(offset.max(end.saturating_sub(MAX_READ))))
        .ok()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    Some(data)
}

/// A record appended to `path` after `offset` carries our message for
/// `qids` (Python `transcript.landed_since`).
pub fn landed_since(path: &Path, offset: u64, qids: &[u64], text: Option<&str>) -> bool {
    let Some(data) = since(path, offset) else {
        return false;
    };
    data.split(|&b| b == b'\n')
        .filter(|raw| raw.windows(7).any(|w| w == b"[inbox]"))
        .any(|raw| {
            record_texts(&String::from_utf8_lossy(raw))
                .texts
                .iter()
                .any(|t| contains(t, qids, text))
        })
}

/// Waits up to `timeout` for the message in any watched transcript (Python
/// `transcript.wait_landed_any`): looked at at once, then every
/// [`INTERVAL`] until the deadline, once more at it.
pub fn wait_landed_any(
    watch: &[Watched],
    qids: &[u64],
    timeout: Duration,
    text: Option<&str>,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if watch
            .iter()
            .any(|(p, off)| landed_since(p, *off, qids, text))
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(INTERVAL);
    }
}

/// The watched transcripts the typed message landed in (Python
/// `service._wait_hits`): after the wait, each read once more so a batch
/// landing in two is seen whole; none when it never showed.
pub fn wait_hits(
    watch: &[Watched],
    qids: &[u64],
    text: Option<&str>,
    timeout: Duration,
) -> BTreeSet<PathBuf> {
    if !wait_landed_any(watch, qids, timeout, text) {
        return BTreeSet::new();
    }
    watch
        .iter()
        .filter(|(p, off)| landed_since(p, *off, qids, text))
        .map(|(p, _)| p.clone())
        .collect()
}
