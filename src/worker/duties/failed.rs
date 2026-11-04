//! The failed pile, reconciled (Python `service.reconcile_failed`,
//! `archive_gone`, `prune_starts`): answers shown as failed (not verified,
//! agent gone, conversation changed, no agent) that DID land in a transcript
//! of their conversation are recorded delivered and verified; answers the
//! conversation was given by its own inbox call are seen; answers already
//! seen lose their stale error; answers gone a day leave the popup.
//!
//! **SAFETY, the forward-family rule (herdr-inbox #53):** only the asker's
//! FORWARD family (the asking session and the sessions it continued in after
//! the question was asked) is looked in. A landing in an older session, one
//! that continued INTO the asker, is no delivery: herdr can resume such an
//! older session in the asking pane, answers typed there never reach the
//! asker, and a reconcile that counted it once marked them verified.

use std::path::{Path, PathBuf};
use std::time::Duration;

use inbox_core::REFERENCE_INDEX;

use super::{Memory, Round};
use crate::claude::evidence::seen_answer;
use crate::claude::landed::{Expected, landed_kinds};
use crate::delivery::expected_segments;
use crate::delivery::plans::question_family;
use crate::store::Question;
use crate::store::sessions::session_transcript;

/// A failed answer is looked for at most this often (Python `RECONCILE_EVERY`).
pub const RECONCILE_EVERY: f64 = 300.0;
/// Failed answers this recent are reconciled (Python `RECONCILE_WINDOW`).
pub const RECONCILE_WINDOW: Duration = Duration::from_secs(7 * 86_400);
/// Gone answers leave the popup after this (Python `GONE_ARCHIVE_AFTER`, operator Q167).
pub const GONE_ARCHIVE_AFTER: Duration = Duration::from_secs(86_400);
/// SessionStart markers are pruned at most this often (Python `PRUNE_STARTS_EVERY`).
pub const PRUNE_STARTS_EVERY: f64 = 3600.0;
/// A start marker older than this is dropped (Python `STARTS_KEEP`).
pub const STARTS_KEEP: Duration = Duration::from_secs(30 * 86_400);

/// Every transcript `q`'s answer counts as landed in: its question
/// family's, each once (Python `_relative_paths`).
pub fn relative_paths(round: &Round<'_, '_>, q: &Question) -> rusqlite::Result<Vec<PathBuf>> {
    let (db, files) = (round.ctx.store.db(), round.ctx.files);
    let mut paths: Vec<PathBuf> = Vec::new();
    for m in question_family(db, files, q)? {
        let recorded = session_transcript(db, &m)?;
        if let Some(p) = files.find(&m, recorded.as_deref())
            && !paths.contains(&p)
        {
            paths.push(p);
        }
    }
    Ok(paths)
}

/// One pass over the failed pile of this worker's server (or of none),
/// backed off per answer.
pub fn reconcile_failed(round: &mut Round<'_, '_>, memory: &mut Memory) -> rusqlite::Result<()> {
    let store = round.ctx.store;
    store.clear_seen_errors()?;
    let t = memory.monotonic();
    let mut rows = Vec::new();
    for q in store.failed_pile(RECONCILE_WINDOW)? {
        let due = memory
            .reconciled_at
            .get(&q.id)
            .is_none_or(|at| t - at >= RECONCILE_EVERY);
        if due && store.server_of(q.id)?.is_none_or(|s| s == round.server) {
            rows.push(q);
        }
    }
    for q in rows {
        memory.reconciled_at.insert(q.id, t);
        // only the asker's forward family: a landing in an older session is no delivery
        let paths = relative_paths(round, &q)?;
        let expected = [(
            q.id as u64,
            Expected {
                segments: expected_segments(&q),
                since: q.answered_at.clone(),
            },
        )]
        .into_iter()
        .collect();
        let mut found = std::collections::BTreeSet::new();
        for p in &paths {
            found.extend(
                landed_kinds(p, &expected)
                    .remove(&(q.id as u64))
                    .unwrap_or_default(),
            );
        }
        if !found.is_empty() {
            store.mark_landed(q.id, found.iter().any(|k| *k != REFERENCE_INDEX))?;
        } else if paths.iter().any(|p| seen(memory, p, &q)) {
            store.mark_seen_elsewhere(q.id, None)?;
        }
    }
    Ok(())
}

fn seen(memory: &mut Memory, path: &Path, q: &Question) -> bool {
    let answer = q.answer.as_deref().unwrap_or("");
    seen_answer(
        &mut memory.evidence,
        path,
        q.id,
        answer,
        q.answered_at.as_deref(),
    )
}

/// Gone a day: out of the popup; `check_answers` keeps them.
pub fn archive_gone(round: &Round<'_, '_>) -> rusqlite::Result<()> {
    round.ctx.store.archive_gone(GONE_ARCHIVE_AFTER).map(drop)
}

/// At most every [`PRUNE_STARTS_EVERY`]: drops SessionStart markers older
/// than [`STARTS_KEEP`] ([`crate::starts::prune_starts`]); the number dropped.
pub fn prune_starts(state_dir: &Path, memory: &mut Memory) -> usize {
    let t = memory.monotonic();
    if memory
        .starts_pruned_at
        .is_some_and(|at| t - at < PRUNE_STARTS_EVERY)
    {
        return 0;
    }
    memory.starts_pruned_at = Some(t);
    crate::starts::prune_starts(state_dir, STARTS_KEEP)
}
