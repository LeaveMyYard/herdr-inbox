//! Verifying a delivery: what a typed answer's landing records (Python
//! `_record_outcome`, used by `deliver_batch` and by the detached
//! `inbox verify` job, `verify_job`), and whether a hook-sent answer reached
//! its conversation (Python `service.verify_hook_deliveries`).
//!
//! The caller looks things up (transcripts, the clock) and writes the
//! outcome; every rule reproduces the `verify_job` and `verify_hook` rows of
//! the delivery decision table (`tests/verify_rows.rs`), and the landing
//! rule the typed `deliver_batch` rows too.

use crate::REFERENCE_INDEX;
use crate::deliver::{NOT_VERIFIED, Outcome};

/// The pending reason of a hook-sent answer not seen in time (Python
/// `HOOK_UNSEEN`).
pub const HOOK_UNSEEN: &str = "hook: not seen in the conversation; trying again";

/// Where a typed answer's text landed (Python `_record_outcome`'s hits and plan).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Landing {
    /// In a transcript of the asker's forward family: delivered.
    pub counted: bool,
    /// Anywhere watched.
    pub anywhere: bool,
    /// The row's state if it landed only in an older session of its conversation.
    pub older_error: Option<String>,
}

/// Python `_record_outcome`: what a typed answer's landing records. `ack`:
/// the message held the full answer; `busy`: it was typed into a busy agent.
/// One of [`Outcome::Delivered`], [`Outcome::Failed`] (an older session's
/// state, or [`NOT_VERIFIED`]) and [`Outcome::BusyMiss`] ([`BUSY_WAIT_IDLE`](crate::deliver::BUSY_WAIT_IDLE)).
pub fn landing(landing: Landing, ack: bool, busy: bool) -> Outcome {
    if landing.counted {
        Outcome::Delivered { ack }
    } else if let Some(older) = landing
        .older_error
        .filter(|e| landing.anywhere && !e.is_empty())
    {
        // Python `hits and plan.get("older_error")`: typed, but never delivered
        Outcome::Failed(older)
    } else if busy {
        // not counted toward the idle cap (MAX_BUSY_ATTEMPTS)
        Outcome::BusyMiss
    } else {
        Outcome::Failed(NOT_VERIFIED.into())
    }
}

/// The lookup of a verify job: where each row's text landed.
pub trait JobLook {
    /// Where row `id`'s text landed, by its own plan: counted in its own
    /// `counting` transcripts, with its own `older_error`. A row without a
    /// plan (an older popup's job) counts every watched transcript and has
    /// no older state.
    fn landing(&mut self, id: i64) -> Landing;
}

/// Python `verify_job`: the outcome of each row of a typed batch, all
/// checked against the one wait for the typed text, each by its own landing
/// (never another row's). `ack` and `busy` are the job's; Python reads them
/// with `job.get("ack", True)` and `job.get("busy")`, so a job without them
/// is `ack = true`, `busy = false` (the caller's parse of the job JSON).
pub fn job(ids: &[i64], ack: bool, busy: bool, look: &mut impl JobLook) -> Vec<(i64, Outcome)> {
    ids.iter()
        .map(|&id| (id, landing(look.landing(id), ack, busy)))
        .collect()
}

/// What becomes of a hook-sent answer this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookOutcome {
    /// Found in the conversation (`hook_verified`): seen unless only the
    /// long-answer reference landed.
    Verified {
        /// Counted as seen.
        ack: bool,
    },
    /// Not there yet, and its time is not up: looked at again next tick.
    Wait,
    /// Not seen in time: back to pending with [`HOOK_UNSEEN`], a miss counted
    /// (`hook_unseen(HOOK_UNSEEN)`).
    Unseen,
    /// Time is up and there is no transcript to look in: left sent but never
    /// counted as seen, like typing without one; no miss (`hook_unseen(None)`).
    SentUnverified,
}

/// The lookups of a hook verification, each made only when it is needed.
pub trait HookLook {
    /// The expected segment indexes found in the conversation's transcripts
    /// (its whole continuation chain), if any.
    fn landed(&mut self) -> Option<Vec<usize>>;
    /// The send is at least `HOOK_VERIFY_SECONDS` (45 s) old, or its time
    /// does not parse.
    fn verify_time_up(&mut self) -> bool;
    /// A transcript of the asking session is known.
    fn transcript_known(&mut self) -> bool;
}

/// Python `verify_hook_deliveries` for one hook-sent answer.
pub fn hook(look: &mut impl HookLook) -> HookOutcome {
    if let Some(segments) = look.landed() {
        return HookOutcome::Verified {
            ack: segments.iter().any(|s| *s != REFERENCE_INDEX),
        };
    }
    if !look.verify_time_up() {
        HookOutcome::Wait
    } else if look.transcript_known() {
        HookOutcome::Unseen
    } else {
        HookOutcome::SentUnverified
    }
}
