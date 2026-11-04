//! Answers marked seen without a verified delivery, re-checked (Python
//! `Store._reconcile_unverified`): an old server's "typed = seen" is looked
//! up in its conversation's transcripts (the asking session and the
//! sessions it continued in, never an older one). Found: verified. Not
//! found: un-seen, so `check_answers` returns it; and only when a
//! transcript was actually read, the delivery was that old server's
//! signature and is recent, it is also put back for delivery. Without a
//! transcript it is only un-seen, never retyped blind.

use std::collections::{BTreeMap, BTreeSet};

use crate::claude::landed::{Expected, landed_kinds};
use crate::claude::{Transcripts, chain};
use crate::delivery::expected_segments;
use crate::store::Store;
use crate::store::outcomes::unverified::{Recheck, Unverified};
use crate::store::sessions::session_transcript;

/// One pass. Python runs it at every maintenance open of the database and
/// again at the start of each tick; the tick calls it ([`super::tick`]),
/// the worker's database open is part 4's (`duties::live`).
pub fn reconcile_unverified(store: &Store, files: &Transcripts) -> rusqlite::Result<()> {
    let (one_off, rows) = store.unverified_rows()?;
    if rows.is_empty() && !one_off {
        return Ok(());
    }
    let mut by_session: Vec<(Option<String>, Vec<i64>)> = Vec::new();
    for Unverified { id, session, .. } in &rows {
        match by_session.iter_mut().find(|(s, _)| s == session) {
            Some((_, ids)) => ids.push(*id),
            None => by_session.push((session.clone(), vec![*id])),
        }
    }
    let (mut found, mut read) = (BTreeSet::new(), BTreeSet::new());
    let db = store.db();
    for (session, qids) in &by_session {
        let mut expected = BTreeMap::new();
        for id in qids {
            if let Some(q) = store.get(*id)? {
                let since = q.answered_at.clone();
                expected.insert(
                    q.id as u64,
                    Expected {
                        segments: expected_segments(&q),
                        since,
                    },
                );
            }
        }
        let first = session.as_deref().unwrap_or("");
        let known = match session {
            Some(s) => session_transcript(db, s)?,
            None => None,
        };
        for s in chain(files, first, known.as_deref()) {
            let recorded = session_transcript(db, &s)?;
            if let Some(path) = files.find(&s, recorded.as_deref()) {
                found.extend(landed_kinds(&path, &expected).into_keys().map(|k| k as i64));
                read.extend(qids.iter().copied());
            }
        }
    }
    let outcomes: Vec<(i64, Recheck)> = rows
        .iter()
        .map(|r| {
            let what = if found.contains(&r.id) {
                Recheck::Found
            } else if r.recent_signature && read.contains(&r.id) {
                Recheck::Requeue
            } else {
                Recheck::Unsee
            };
            (r.id, what)
        })
        .collect();
    store.apply_unverified(one_off, &outcomes)
}
