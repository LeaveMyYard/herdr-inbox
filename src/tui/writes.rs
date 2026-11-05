//! The model's database writes, made on the popup's thread as Python makes
//! them (Python `store.set_draft`, `service.close(…, "dismissed")`,
//! `store.hide_failed`, `alerts.acknowledge`): a draft, a dismissal, a
//! hidden ⚠ answer, an acknowledged alert. Each is one short statement; the
//! pane marks they change go to the background queue ([`Marks`]).

use inbox_core::popup::Effect;

use crate::service::answer::close_marking;
use crate::store::Store;
use crate::store::alerts::Effect as AlertEffect;
use crate::tui::marks::Marks;
use crate::tui::snapshot::convert::detail_json;

/// Makes `effect`'s write; `Ok(false)` if it is no write. `Err`: the
/// database failed and nothing more of the operator's action may follow.
pub fn write(store: &Store, effect: &Effect, marks: &Marks) -> rusqlite::Result<bool> {
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
        _ => return Ok(false),
    }
    Ok(true)
}
