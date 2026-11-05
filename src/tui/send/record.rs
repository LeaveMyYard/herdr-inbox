//! Recording what the operator sent (Python `_send`'s `work`): the answers
//! written to the database, before anyone delivers them. **SAFETY**: this is
//! where an answer becomes a question's answer.
//!
//! - A lone question: its answer as the draft, then answered (Python
//!   `service.answer(…, choices=…, notes=…, deliver=False)`).
//! - A group's Submit: its drafted open questions answered (Python
//!   `service.submit(…, deliver=False)`).
//! - Retry / Send now: nothing is written; the answer is handed to delivery
//!   again with force, unless another process holds a live claim on it right
//!   now (Python's `AlreadySending`): it is that sender's.

use inbox_core::popup::Send;

use crate::service::answer::{answer_detail, submit_marking};
use crate::store::claims::is_live;
use crate::store::{Question, Store};
use crate::tui::snapshot::convert::detail_json;

/// What a recorded send hands to delivery.
#[derive(Debug)]
pub struct Recorded {
    /// The answers, as they stand now.
    pub done: Vec<Question>,
    /// How many it was asked to send.
    pub wanted: usize,
    /// Delivered with force (the operator's Retry / Send now).
    pub force: bool,
}

/// Why nothing was recorded.
#[derive(Debug)]
pub enum Refused {
    /// Retry on an answer another process is delivering right now.
    AlreadySending(i64),
    /// The database failed.
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for Refused {
    fn from(err: rusqlite::Error) -> Refused {
        Refused::Db(err)
    }
}

/// Records `send` in `store`; each answered pane's marks go to `marks`.
pub fn record(
    store: &Store,
    send: &Send,
    marks: &mut dyn FnMut(Option<&str>),
) -> Result<Recorded, Refused> {
    let (done, wanted, force) = match send {
        Send::Answer { qid, detail } => {
            let done = match detail {
                Some(d) => answer_detail(store, *qid, &detail_json(d), None, marks)?,
                None => None, // nothing picked, nothing typed: Python's answer records nothing
            };
            (done.into_iter().collect(), 1, false)
        }
        Send::Submit { qids } => (submit_marking(store, qids, None, marks)?, qids.len(), false),
        Send::Retry { qid } => {
            let q = store.get(*qid)?;
            let err = q.as_ref().and_then(|q| q.delivery_error.as_deref());
            if err.is_some_and(is_live) {
                return Err(Refused::AlreadySending(*qid)); // its sender is at it right now
            }
            (q.into_iter().collect(), 1, true)
        }
    };
    Ok(Recorded {
        done,
        wanted,
        force,
    })
}
