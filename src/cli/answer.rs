//! `inbox answer ID TEXT...` and `inbox dismiss ID` (Python
//! `cli._command`). **SAFETY**: `answer` decides whether an answer is
//! typed into the asking conversation or recorded as already there.

use super::args::parse::Args;
use super::args::question_id;
use super::{Io, warner};
use crate::service::Delivery;
use crate::service::answer::{answer as answer_now, close};
use crate::service::own::answered_by_own_conversation;
use crate::store::Store;
use crate::surface::markers::Markers;

/// Answers question `id` with the words of `text` joined by spaces, then
/// delivers it, unless this process runs in the conversation that asked:
/// that agent has the answer already (it relays the operator's decision
/// from chat), so it is recorded as landed but not seen (a subagent shares
/// the session id; the main thread must still get it from check_answers).
/// 1 when the question is not open.
pub fn answer(store: &Store, args: &Args, io: &mut Io<'_>) -> rusqlite::Result<i32> {
    let id = parsed_id(args);
    let text = args.values[1].join(" ");
    io.delivery.ensure_worker(None); // it retries whatever this cannot deliver right away
    let asked = store.get(id)?;
    let own = match &asked {
        Some(q) if q.status == "open" => {
            answered_by_own_conversation(store, io.files, io.state_dir, io.me, q)?
        }
        _ => false,
    };
    let done = with_markers(store, io, |m, d| {
        answer_now(store, id, &text, (!own).then_some(d), m)
    })?;
    let Some(q) = done else {
        let _ = writeln!(io.err, "Q{id} is not open");
        return Ok(1);
    };
    if own {
        store.mark_landed(q.id, false)?;
        let _ = writeln!(
            io.out,
            "Q{} answered by its own conversation: nothing to deliver",
            q.id
        );
        return Ok(0);
    }
    let state = match &q.delivered_at {
        Some(_) => format!("delivered to {}", q.pane_id.as_deref().unwrap_or("None")),
        None => format!(
            "not delivered: {}",
            q.delivery_error.as_deref().unwrap_or("None")
        ),
    };
    let _ = writeln!(io.out, "Q{} answered, {state}", q.id);
    Ok(0)
}

/// Closes question `id` without an answer; 1 when it is not open.
pub fn dismiss(store: &Store, args: &Args, io: &mut Io<'_>) -> rusqlite::Result<i32> {
    let id = parsed_id(args);
    let closed = with_markers(store, io, |m, _| close(store, id, "dismissed", None, m))?;
    if !closed {
        let _ = writeln!(io.err, "Q{id} is not open");
        return Ok(1);
    }
    let _ = writeln!(io.out, "Q{id} dismissed");
    Ok(0)
}

/// Runs `f` with the marks of the panes it changes (warnings to stderr) and
/// the delivery.
fn with_markers<T>(
    store: &Store,
    io: &mut Io<'_>,
    f: impl FnOnce(&mut Markers<'_>, &mut dyn Delivery) -> T,
) -> T {
    let mut warn = warner(&mut *io.err);
    let mut markers = Markers {
        store,
        db_path: io.db_path,
        herdr: &mut *io.herdr,
        config: &mut *io.config,
        lock_wait: io.lock_wait,
        warn: &mut warn,
    };
    f(&mut markers, &mut *io.delivery)
}

/// The `id` argument, which the parser already checked.
fn parsed_id(args: &Args) -> i64 {
    args.values[0]
        .first()
        .and_then(|v| question_id(v))
        .unwrap_or_default()
}
