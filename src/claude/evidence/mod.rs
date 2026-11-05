//! Whether a conversation was GIVEN an answer by its own inbox calls
//! (Python `transcript.evidence`, `seen_answer`; review #44 rounds 1-3):
//! `check_answers`' JSON with that exact answer, or the CLI's own
//! `Q7 answered` line after the conversation ran `inbox answer 7` itself.
//! A tool result counts only through the tool call it answers; quoted
//! output (a terminal read, a log, an echo) and agent-written text never do.
//!
//! **SAFETY**: a yes marks the answer seen and it is never typed again, so
//! a false one loses an answer silently.
//!
//! [`Evidence`] indexes each transcript incrementally: an append is read
//! from the indexed offset (its last bytes checked unchanged), anything else
//! (a rewrite, even of the same size) from the start.

mod records;
pub mod shell;
pub mod shlex;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use serde_json::Value;

use super::landed::after;
use crate::text::squash;
use records::{Use, answers_in, tool_results, tool_uses};

/// Transcripts indexed at most (Python `CACHE_MAX`, LRU).
pub const CACHE_MAX: usize = 256;
const EDGE: u64 = 64;

/// What a conversation's own call gave it for one question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Given {
    /// It answered the question itself (Python `OWN`).
    Own,
    /// `check_answers` returned this answer.
    Answer(String),
}

/// Per question: `(record time, what it was given)`.
pub type Found = BTreeMap<i64, Vec<(Option<String>, Given)>>;

type Stat = (u64, i64, u64);

#[derive(Clone, Default)]
struct Indexed {
    stat: Option<Stat>,
    offset: u64,
    edge: Vec<u8>,
    found: Found,
    uses: HashMap<String, Use>,
}

/// The evidence of each transcript, kept per process (Python
/// `_evidence_cache`).
#[derive(Default)]
pub struct Evidence {
    cache: HashMap<String, Indexed>,
    order: VecDeque<String>,
    /// Bytes read from transcripts (the tests check each is read once).
    pub bytes_read: u64,
}

fn stat_of(path: &Path) -> Option<Stat> {
    let m = std::fs::metadata(path).ok()?;
    let ns = m
        .mtime()
        .saturating_mul(1_000_000_000)
        .saturating_add(m.mtime_nsec());
    Some((m.len(), ns, m.ino()))
}

impl Evidence {
    /// What `path`'s conversation was given, by question.
    pub fn of(&mut self, path: &Path) -> Found {
        let key = path.to_string_lossy().into_owned();
        let Some(stat) = stat_of(path) else {
            return Found::new();
        };
        let cached = self.cache.get(&key).cloned();
        if let Some(c) = &cached
            && c.stat == Some(stat)
        {
            return c.found.clone();
        }
        match self.index(path, stat, cached) {
            Some(ix) => {
                let found = ix.found.clone();
                self.remember(key, ix);
                found
            }
            None => Found::new(),
        }
    }

    fn remember(&mut self, key: String, ix: Indexed) {
        self.order.retain(|k| *k != key);
        self.order.push_back(key.clone());
        self.cache.insert(key, ix);
        while self.order.len() > CACHE_MAX {
            if let Some(old) = self.order.pop_front() {
                self.cache.remove(&old);
            }
        }
    }

    fn index(&mut self, path: &Path, stat: Stat, cached: Option<Indexed>) -> Option<Indexed> {
        let mut fh = File::open(path).ok()?;
        let grow = match &cached {
            Some(c) if c.stat.is_some_and(|s| s.2 == stat.2) && c.offset < stat.0 => {
                let n = c.edge.len() as u64;
                fh.seek(SeekFrom::Start(c.offset.saturating_sub(n))).ok()?;
                let mut edge = vec![0; c.edge.len()];
                fh.read_exact(&mut edge).is_ok() && edge == c.edge
            }
            _ => false,
        };
        let mut ix = match cached {
            Some(c) if grow => c,
            _ => Indexed::default(),
        };
        let start = ix.offset;
        fh.seek(SeekFrom::Start(start)).ok()?;
        let mut data = Vec::new();
        fh.by_ref()
            .take(stat.0.saturating_sub(start))
            .read_to_end(&mut data)
            .ok()?;
        self.bytes_read += data.len() as u64;
        ix.stat = Some(stat);
        let Some(end) = data.iter().rposition(|&b| b == b'\n') else {
            return Some(ix); // no whole line yet: nothing indexed
        };
        ix.offset = start + end as u64 + 1;
        let from = ix.offset.saturating_sub(EDGE);
        fh.seek(SeekFrom::Start(from)).ok()?;
        let mut edge = Vec::new();
        fh.by_ref()
            .take(ix.offset - from)
            .read_to_end(&mut edge)
            .ok()?;
        ix.edge = edge;
        for raw in data[..end].split(|&b| b == b'\n') {
            read_line(raw, &mut ix);
        }
        Some(ix)
    }
}

fn has(raw: &[u8], needle: &[u8]) -> bool {
    raw.windows(needle.len()).any(|w| w == needle)
}

/// One transcript line into the index (Python `evidence`'s loop body).
fn read_line(raw: &[u8], ix: &mut Indexed) {
    let is_use = has(raw, b"\"tool_use\"") && has(raw, b"inbox");
    let is_result = has(raw, b"\"tool_result\"") && has(raw, b"answer");
    if !is_use && !is_result {
        return;
    }
    let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(raw) else {
        return;
    };
    if is_use {
        for (id, what) in tool_uses(&record) {
            ix.uses.insert(id, what);
        }
        return;
    }
    let ts = record
        .get("timestamp")
        .and_then(Value::as_str)
        .map(String::from);
    for (used, text) in tool_results(&record) {
        match used.as_ref().and_then(|u| ix.uses.get(u)) {
            Some(Use::Own(qids)) => {
                for &qid in qids {
                    if shell::own_answer_succeeded(&text, qid) {
                        ix.found
                            .entry(qid)
                            .or_default()
                            .push((ts.clone(), Given::Own));
                    }
                }
            }
            Some(Use::Read) if text.contains("\"answer\"") => {
                let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let mut pairs = Vec::new();
                answers_in(&parsed, &mut pairs);
                for (qid, answer) in pairs {
                    ix.found
                        .entry(qid)
                        .or_default()
                        .push((ts.clone(), Given::Answer(answer)));
                }
            }
            _ => {}
        }
    }
}

/// The conversation of `path` was given `answer` to `qid` by a tool, at or
/// after `since` (Python `seen_answer`): never a substring, never before the
/// answer's time.
pub fn seen_answer(
    ev: &mut Evidence,
    path: &Path,
    qid: i64,
    answer: &str,
    since: Option<&str>,
) -> bool {
    let want = squash(answer);
    ev.of(path).get(&qid).is_some_and(|given| {
        given.iter().any(|(ts, g)| {
            after(ts.as_deref(), since)
                && match g {
                    Given::Own => true,
                    Given::Answer(a) => squash(a) == want,
                }
        })
    })
}

#[cfg(test)]
mod tests;
