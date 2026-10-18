//! Has an answer already landed? (Python `service.landed_across`.)
//!
//! Looked for in every transcript of each question's forward family (a
//! delivery that landed in the OLD transcript just before a continuation is
//! found, never sent twice), plus extra paths such as the hook payload's own
//! transcript. Never in an older session's transcript: a landing there is no
//! delivery.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use rusqlite::Connection;

use super::{expected_segments, forward_family};
use crate::claude::Transcripts;
use crate::claude::landed::{Expected, landed_kinds};
use crate::store::Question;
use crate::store::sessions::session_transcript;

/// Who asked a question and when: questions sharing it share a forward family.
type Asked<'q> = (Option<&'q str>, &'q str);

/// For each of `qs` that landed: the indexes of its expected strings found
/// (see [`expected_segments`]).
pub fn landed_across(
    db: &Connection,
    files: &Transcripts,
    qs: &[Question],
    extra: &[Option<PathBuf>],
) -> rusqlite::Result<BTreeMap<i64, BTreeSet<usize>>> {
    // path -> the questions to look for there, both in first-seen order
    let mut paths: Vec<(PathBuf, Vec<&Question>)> = Vec::new();
    let mut groups: Vec<(Asked<'_>, Vec<&Question>)> = Vec::new();
    for q in qs {
        let key = (q.session_id.as_deref(), q.created_at.as_str());
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(q),
            None => groups.push((key, vec![q])),
        }
    }
    for ((session, created), items) in &groups {
        for s in forward_family(db, files, *session, Some(created))? {
            let known = session_transcript(db, &s)?;
            if let Some(p) = files.find(&s, known.as_deref()) {
                add(&mut paths, p, items);
            }
        }
    }
    let all: Vec<&Question> = qs.iter().collect();
    for p in extra.iter().flatten() {
        add(&mut paths, p.clone(), &all);
    }
    let mut found: BTreeMap<i64, BTreeSet<usize>> = BTreeMap::new();
    for (path, items) in &paths {
        let expected: BTreeMap<u64, Expected> = items
            .iter()
            .map(|q| {
                let e = Expected {
                    segments: expected_segments(q),
                    since: q.answered_at.clone(),
                };
                (u64::try_from(q.id).unwrap_or(0), e)
            })
            .collect();
        for (id, kinds) in landed_kinds(path, &expected) {
            found.entry(id as i64).or_default().extend(kinds);
        }
    }
    Ok(found)
}

/// Adds `items` to `path`'s list, keeping the first-seen order of paths.
fn add<'q>(paths: &mut Vec<(PathBuf, Vec<&'q Question>)>, path: PathBuf, items: &[&'q Question]) {
    match paths.iter_mut().find(|(p, _)| *p == path) {
        Some((_, list)) => list.extend_from_slice(items),
        None => paths.push((path, items.to_vec())),
    }
}
