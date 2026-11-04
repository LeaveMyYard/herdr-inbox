//! Answers handed over as hook context, verified (Python
//! `service.verify_hook_deliveries`, 0.9.0): found in the conversation's
//! transcripts (its whole forward family), verified and seen; not found
//! within `HOOK_VERIFY_SECONDS`, back to pending with a miss counted. The
//! decision is `inbox_core::verify::hook`'s (the `verify_hook` rows of the
//! delivery decision table).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_agent_kit::agent_session::claude::wire::ts;
use inbox_core::verify::{HOOK_UNSEEN, HookLook, HookOutcome, hook};
use serde_json::Value;

use super::Round;
use crate::delivery::landed_across;
use crate::store::Question;
use crate::store::sessions::session_transcript;

/// A hook-sent answer must show in the transcript within this (Python `HOOK_VERIFY_SECONDS`).
pub const HOOK_VERIFY_SECONDS: Duration = Duration::from_secs(45);

struct Look<'r, 'x, 'a> {
    round: &'r Round<'x, 'a>,
    q: &'r Question,
    sent_at: &'r str,
    err: Option<rusqlite::Error>,
}

impl Look<'_, '_, '_> {
    fn ok<T: Default>(&mut self, r: rusqlite::Result<T>) -> T {
        r.unwrap_or_else(|e| {
            self.err.get_or_insert(e);
            T::default()
        })
    }
}

impl HookLook for Look<'_, '_, '_> {
    fn landed(&mut self) -> Option<Vec<usize>> {
        let ctx = &self.round.ctx;
        let found = landed_across(ctx.store.db(), ctx.files, std::slice::from_ref(self.q), &[]);
        self.ok(found)
            .remove(&self.q.id)
            .map(|k| k.into_iter().collect())
    }

    fn verify_time_up(&mut self) -> bool {
        // Python: an unparseable time counts as HOOK_VERIFY_SECONDS old
        let Some(sent) = ts(&Value::String(self.sent_at.to_string())) else {
            return true;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        now - sent.secs() >= HOOK_VERIFY_SECONDS.as_secs_f64()
    }

    fn transcript_known(&mut self) -> bool {
        let ctx = &self.round.ctx;
        let Some(session) = self.q.session_id.as_deref().filter(|s| !s.is_empty()) else {
            return false;
        };
        let known = session_transcript(ctx.store.db(), session);
        let known = self.ok(known);
        ctx.files.find(session, known.as_deref()).is_some()
    }
}

/// One pass over this server's hook-sent, unverified answers.
pub fn verify_hook_deliveries(round: &mut Round<'_, '_>) -> rusqlite::Result<()> {
    let store = round.ctx.store;
    for (q, sent_at) in store.hook_unverified(round.server)? {
        let mut look = Look {
            round,
            q: &q,
            sent_at: &sent_at,
            err: None,
        };
        let outcome = hook(&mut look);
        if let Some(e) = look.err {
            return Err(e);
        }
        match outcome {
            HookOutcome::Verified { ack } => store.hook_verified(q.id, ack)?,
            HookOutcome::Wait => {}
            HookOutcome::Unseen => drop(store.hook_unseen(q.id, &sent_at, Some(HOOK_UNSEEN))?),
            HookOutcome::SentUnverified => drop(store.hook_unseen(q.id, &sent_at, None)?),
        }
    }
    Ok(())
}
