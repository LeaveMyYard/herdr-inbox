//! Hook delivery (Python `hook.deliver`): a session's waiting answers as the
//! hook's `additionalContext`, the same `[inbox] …` text typing would send,
//! one message per firing.
//!
//! Each answer is claimed (CAS), checked against every transcript of its
//! conversation (a late landing is recorded, never sent twice), and recorded
//! as hook-sent BEFORE the text is printed. The worker verifies it in the
//! transcript later, or puts it back to pending.

use std::path::Path;
use std::time::{Duration, SystemTime};

use herdr_agent_kit::herdr_api::{MAX_ARG, clean};
use herdr_agent_kit::internal::inbox_contract::delivery::delivery_messages;
use inbox_core::hook::{self, Candidate, Carry, Gate, Message};

use super::payload::Payload;
use crate::claude::Transcripts;
use crate::delivery::landed_across;
use crate::delivery::segments::answered;
use crate::store::candidates::has_candidates;
use crate::store::liveness::{Stamp, touch};
use crate::store::sessions::record_session;
use crate::store::{Claim, MAX_HOOK_MISSES, OpenError, Question, Store};

/// SQLite's busy timeout for the hook's writes (Python `RECORD_TIMEOUT`).
pub const RECORD_TIMEOUT: Duration = Duration::from_millis(500);

/// No cap on the printed text: `herdr.clean(text, limit=10**9)`.
const NO_LIMIT: usize = 1_000_000_000;

/// The text to print, or `None`. `liveness`: stamp `hook_seen_at` (the
/// PostToolBatch and Stop hooks; the rewake watcher stamps its own column).
/// A database error anywhere means nothing is printed, as when Python's
/// hook raised; a claim taken before it expires after 60 s.
pub fn deliver(
    db_path: &Path,
    files: &Transcripts,
    payload: &Payload,
    liveness: bool,
) -> Option<String> {
    let own = hook::own_session(payload.session_id.as_deref(), payload.agent_id);
    let session = payload.session_id.as_deref().unwrap_or_default();
    if own && liveness {
        touch(db_path, session, Stamp::HookSeen, SystemTime::now());
    }
    // an open error other than the schema (locked, unreadable) also means nothing now
    let mut store = None;
    let gate = hook::gate(
        own,
        || has_candidates(db_path, session, files),
        || match Store::open(db_path, RECORD_TIMEOUT) {
            Ok(opened) => {
                store = Some(opened);
                true
            }
            Err(OpenError::NotReady(_) | OpenError::Sqlite(_)) => false,
        },
    );
    if gate != Gate::Carry {
        return None; // not this session's own hook, nothing waiting, or no usable database
    }
    carry(
        store.as_ref()?,
        files,
        session,
        payload.transcript_path.as_deref(),
    )
    .ok()
    .flatten()
}

/// The steps after the gates, under the store.
fn carry(
    store: &Store,
    files: &Transcripts,
    session: &str,
    transcript: Option<&str>,
) -> rusqlite::Result<Option<String>> {
    record_session(store.db(), session, transcript)?;
    let mut claimed: Vec<(Question, Claim)> = Vec::new();
    for q in store.hook_candidates(session, MAX_HOOK_MISSES, files)? {
        if let Some(claim) = store.claim_delivery(q.id)? {
            claimed.push((q, claim));
        }
    }
    if claimed.is_empty() {
        return Ok(None);
    }
    let own = files.find(session, transcript);
    let qs: Vec<Question> = claimed.iter().map(|(q, _)| q.clone()).collect();
    let landed = landed_across(store.db(), files, &qs, &[own])?;
    let mut waiting: Vec<&(Question, Claim)> = Vec::new();
    for item in &claimed {
        let c = Candidate {
            claimed: true,
            landed: landed.get(&item.0.id),
            in_this_message: false,
        };
        match hook::carry(&c) {
            Carry::Landed { acknowledge } => {
                store.record_delivery(item.0.id, None, acknowledge, Some(&item.1), true)?
            }
            _ => waiting.push(item),
        }
    }
    let Some(first) = first_message(waiting.iter().map(|(q, _)| q)) else {
        return Ok(None);
    };
    let mut sent: Vec<&Question> = Vec::new();
    for (q, claim) in waiting {
        let c = Candidate {
            claimed: true,
            landed: None,
            in_this_message: first.ids.contains(&(q.id as u64)),
        };
        match hook::carry(&c) {
            Carry::NextFiring => store.release_claim(q.id, claim, q.delivery_error.as_deref())?,
            Carry::Send => {
                if store.record_hook_delivery(q.id, claim)? {
                    sent.push(q);
                }
            }
            Carry::Landed { .. } | Carry::NotClaimed => {}
        }
    }
    let text = match hook::message(sent.len(), first.ids.len()) {
        Message::Nothing => return Ok(None),
        Message::AsBuilt => first.text,
        Message::Rebuilt => first_message(sent.into_iter())
            .map(|m| m.text)
            .unwrap_or_default(),
    };
    Ok(Some(clean(&text, NO_LIMIT)))
}

/// The first message of these answers, cut at herdr's argument cap (Python
/// `service.delivery_messages(qs)[0]`).
fn first_message<'q>(
    qs: impl Iterator<Item = &'q Question>,
) -> Option<herdr_agent_kit::internal::inbox_contract::delivery::Message> {
    let qs: Vec<&Question> = qs.collect();
    let shown: Vec<_> = qs.iter().map(|q| answered(q)).collect();
    delivery_messages(&shown, MAX_ARG).into_iter().next()
}
