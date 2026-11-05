//! The model's database writes, made on the popup's thread as Python makes
//! them (Python `store.set_draft`, `service.close(…, "dismissed")`,
//! `store.hide_failed`, `alerts.acknowledge`): a draft, a dismissal, a
//! hidden ⚠ answer, an acknowledged alert. Each is one short statement; the
//! pane marks they change go to the background queue ([`Marks`]). [`remote`]
//! has the writes on the remote machines' mirror.

pub mod remote;

use inbox_core::popup::Effect;

use crate::service::answer::close_marking;
use crate::store::Store;
use crate::store::alerts::Effect as AlertEffect;
use crate::tui::marks::Marks;
use crate::tui::snapshot::convert::detail_json;

pub use remote::RemoteEnv;

/// What became of a write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It was made (or the effect is no write).
    Done,
    /// A remote write was not made, and the model must hear it: the operator's
    /// notice, if any
    /// ([`Event::RemoteRefused`](inbox_core::popup::Event::RemoteRefused)).
    /// Nothing more of the action may follow.
    Refused(Option<String>),
}

/// Makes `effect`'s write. `Err`: the database failed and nothing more of
/// the operator's action may follow.
pub fn write(
    store: &Store,
    effect: &Effect,
    marks: &Marks,
    env: RemoteEnv,
) -> rusqlite::Result<Outcome> {
    match effect {
        Effect::Draft { qid, detail } => {
            // not open any more: nothing stored, as Python's rowcount (the model said so)
            store.set_draft(*qid, detail.as_ref().map(detail_json).as_ref())?;
        }
        Effect::Dismiss { qid } => {
            close_marking(store, *qid, "dismissed", None, &mut |p| marks.push(p))?;
        }
        Effect::Hide { qid } => store.hide_failed(*qid)?,
        Effect::Ack { alert } => {
            for fx in store.acknowledge_alert(*alert)?.unwrap_or_default() {
                if let AlertEffect::Markers(pane) = fx {
                    marks.push(pane.as_deref());
                }
            }
        }
        other => return Ok(remote::write(store, other, env)),
    }
    Ok(Outcome::Done)
}
