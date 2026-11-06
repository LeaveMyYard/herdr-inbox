//! Why a row or a question is gone: the `closed` word of a `stale` refusal.

use crate::store::Store;
use crate::store::alerts::Alert;
use crate::store::question::Question;

/// The word for a question that left `open`. `None`: another status, or open.
pub fn question_word(q: &Question) -> Option<&'static str> {
    match q.status.as_str() {
        "answered" => Some("answered"),
        "dismissed" => Some("dismissed"),
        "withdrawn" => Some("withdrawn"),
        _ => None,
    }
}

/// The word for a group with no open question left: the one a `--question`
/// names, else its last, `removed` if nothing is there.
pub fn group_word(
    store: &Store,
    group: i64,
    named: &[i64],
) -> rusqlite::Result<Option<&'static str>> {
    let rows = store.group(group, true)?;
    let shown = rows.iter().find(|q| named.contains(&q.id)).or(rows.last());
    Ok(match shown {
        Some(q) => question_word(q),
        None => Some("removed"),
    })
}

/// Why `q` is no `failed` row any more: `Some(None)` when it is none, with no
/// word for it. `None` when it is one: answered, not delivered, not fetched,
/// not hidden. A live claim and the 60 s wait do not matter here.
pub fn failed_gone(store: &Store, q: &Question) -> rusqlite::Result<Option<Option<&'static str>>> {
    Ok(if q.status != "answered" {
        Some(question_word(q))
    } else if q.delivered_at.is_some() {
        Some(Some("delivered"))
    } else if q.acknowledged_at.is_some() {
        Some(Some("fetched"))
    } else if store.hidden(q.id)? {
        Some(Some("hidden"))
    } else {
        None
    })
}

/// The word for an alert no longer open.
pub fn alert_word(a: &Alert) -> Option<&'static str> {
    match a.status.as_str() {
        "acked" => Some("acked"),
        "cleared" => Some("cleared"),
        "withdrawn" => Some("withdrawn"),
        _ => None,
    }
}
