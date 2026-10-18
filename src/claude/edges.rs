//! Every continuation a transcript EVER made (Python
//! `transcript.continued_edges`): a resumed old session keeps its
//! `continued-in` record mid-file. For following only the continuations made
//! after a moment (`delivery::forward_family`), never for ownership.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use herdr_agent_kit::agent_session::SessionId;
use herdr_agent_kit::agent_session::claude::wire::Line;

use super::continued::in_subagents_dir;

/// One `continued-in` record: the target session and the record's time
/// (epoch seconds), if it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// The canonical id of the session continued in.
    pub target: String,
    /// The record's own timestamp.
    pub at: Option<f64>,
}

/// The `continued-in` records of the main conversation in `path` (never an
/// `isSidechain` record, never a subagent's file), read line by line over the
/// complete lines; each distinct (target, time) once, in (target, time)
/// order with an undated record first, as Python sorts them. Empty when the
/// file cannot be read.
pub fn continued_edges(path: &Path) -> Vec<Edge> {
    if in_subagents_dir(path) {
        return Vec::new();
    }
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let own = path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(SessionId::parse);
    let mut reader = BufReader::new(file);
    let mut found: Vec<Edge> = Vec::new();
    let mut raw = Vec::new();
    loop {
        raw.clear();
        match reader.read_until(b'\n', &mut raw) {
            Ok(0) | Err(_) => break,
            Ok(_) if raw.last() != Some(&b'\n') => break, // a partial last line: read next time
            Ok(_) => {}
        }
        if !raw.windows(12).any(|w| w == b"continued-in") {
            continue;
        }
        if let Some(edge) = edge_of(&raw, own.as_ref())
            && !found.contains(&edge)
        {
            found.push(edge);
        }
    }
    found.sort_by(|a, b| {
        (a.target.as_str(), a.at.unwrap_or(0.0))
            .partial_cmp(&(b.target.as_str(), b.at.unwrap_or(0.0)))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    found
}

fn edge_of(raw: &[u8], own: Option<&SessionId>) -> Option<Edge> {
    let line = Line::parse(raw)?;
    if line.kind() != Some("continued-in") || line.is_sidechain() {
        return None;
    }
    let target = line
        .get("continuedInSessionId")
        .and_then(SessionId::from_value)?;
    if Some(&target) == own {
        return None;
    }
    Some(Edge {
        target: target.as_str().to_owned(),
        at: line.ts().map(|t| t.secs()),
    })
}

#[cfg(test)]
mod tests;
