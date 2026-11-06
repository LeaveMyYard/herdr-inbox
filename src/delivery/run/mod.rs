//! One delivery pass (Python `service.deliver_batch` without its
//! worker-starting preamble, which is the worker's, P5-E): per pane, the
//! decision of `inbox_core::deliver::decide`, its lookups made through
//! `look::PaneLook` and each outcome written to the store (or its
//! verify-later job handed over) the moment it is decided.

mod look;
mod record;
mod relocate;
mod seat;
mod typed;
mod unreadable;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use herdr_agent_kit::herdr_api::blocking::Herdr;
use inbox_core::deliver::{Mode, REQUEUED, Waiting, decide};
use serde_json::Value;

use crate::claude::Transcripts;
use crate::delivery::terminal::Terminals;
use crate::store::{Claim, Question, Store};

pub use relocate::{MOVED, session_pane, tab_name};
pub use seat::{Seat, by_seat, seat_of};
pub use unreadable::{UNREADABLE_NOTIFY_AFTER, Unreadable};

/// How long a typed message is waited for (Python `VERIFY_SECONDS`).
pub const VERIFY_SECONDS: Duration = Duration::from_secs(15);
/// A busy agent's answer waits this long before it is typed anyway (Python `BUSY_GRACE`).
pub const BUSY_GRACE: Duration = Duration::from_secs(20);
/// A working session counts as hook-delivered only if its hook fired this recently
/// (Python `HOOK_LIVE`).
pub const HOOK_LIVE: Duration = Duration::from_secs(600);

/// What a pass works with.
pub struct Ctx<'a> {
    /// The database.
    pub store: &'a Store,
    /// herdr.
    pub herdr: &'a Herdr,
    /// herdr-core's terminals.
    pub core: &'a dyn Terminals,
    /// Claude Code's transcripts.
    pub files: &'a Transcripts,
    /// The database's directory (rewake locks, SessionStart markers).
    pub state_dir: &'a Path,
    /// Shows a notification (Python `later(lambda: herdr.notify(...))`).
    pub notify: &'a mut dyn FnMut(&str, &str),
    /// The once-per-pane unreadable-input-box notice's state (per process).
    pub unreadable: &'a mut Unreadable,
    /// How long a typed message is waited for.
    pub verify_wait: Duration,
}

/// A typed batch whose check runs in a detached `inbox verify` (the popup's
/// `verify_later`): the job as Python's `verify_later` receives it.
pub type VerifyLater<'a> = &'a mut dyn FnMut(Value);

/// Delivers `qs` (Python `deliver_batch` after the foreign-server split):
/// one pass per pane or terminal, in the order they first appear. `Err`: the
/// database failed. As in Python, the pass stops there: what was written
/// before stays (each outcome is written as it is decided), nothing more is
/// typed or written, and the rows it claimed since keep their claim until it
/// goes stale. Whether the pass verifies later is whether `verify_later` is
/// given (Python's `verify_later is not None`); `mode.verify_later` is
/// ignored, so a typed answer is never left without an outcome or a job.
pub fn deliver_batch(
    ctx: &mut Ctx<'_>,
    qs: &[Question],
    mode: Mode,
    mut verify_later: Option<VerifyLater<'_>>,
) -> rusqlite::Result<()> {
    let mode = Mode {
        verify_later: verify_later.is_some(),
        ..mode
    };
    for (seat, items) in by_seat(ctx.store, qs)? {
        let job: Option<&mut dyn FnMut(Value)> = match verify_later.as_mut() {
            Some(f) => Some(&mut **f),
            None => None,
        };
        deliver_seat(ctx, &seat, &items, mode, job)?;
    }
    Ok(())
}

fn deliver_seat(
    ctx: &mut Ctx<'_>,
    seat: &Seat,
    items: &[&Question],
    mode: Mode,
    verify_later: Option<&mut dyn FnMut(Value)>,
) -> rusqlite::Result<()> {
    let waiting: Vec<Waiting> = items
        .iter()
        .map(|q| Waiting {
            id: q.id,
            session: q.session_id.clone(),
            requeued: q.delivery_error.as_deref() == Some(REQUEUED),
        })
        .collect();
    let (pane, core) = match seat {
        Seat::Terminal(terminal) => (Some(terminal.as_str()), true),
        Seat::Pane(pane) => (pane.as_deref(), false),
        Seat::Nowhere => return Ok(()),
    };
    // a core terminal is never forced, whoever asks: not its waits, not `hc send`
    let mode = Mode {
        force: mode.force && !core,
        ..mode
    };
    // a shorter-lived trait object for the pass (a `&mut` is invariant)
    let later = verify_later.map(|f| f as &mut dyn FnMut(Value));
    let mut look = look::PaneLook::new(ctx, pane.unwrap_or(""), core, items, mode, later);
    // the outcomes are written as they are decided (`Look::record`)
    decide(pane, &waiting, mode, &mut look);
    look.finish().map_or(Ok(()), Err)
}

/// The claims a pass took, per question.
pub type Claims = BTreeMap<i64, Claim>;
