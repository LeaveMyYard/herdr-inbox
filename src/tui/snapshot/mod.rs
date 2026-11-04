//! The database as the popup model reads it ([`Snapshot`], Python
//! `InboxApp._live`, `store.group`, `alerts.open_alerts`): read at open, at
//! every poll and before every key, on the popup's own thread.
//!
//! The model needs every row it may show or name, and Python reads more
//! than the open ones: a closed row's status says why its placeholder
//! closed. So besides the open questions, the undelivered answers and the
//! open alerts, a read takes the whole of every group the popup has on
//! screen or in its list, every ⚠ answer it lists and every alert it lists,
//! closed or not ([`Shown`]). A row missing from the read was removed.
//!
//! - [`convert`]: a store row as the model's.
//! - [`busy`]: how long each pending answer has waited (the renderer's
//!   "will send in N s").

pub mod busy;
pub mod convert;

use std::collections::BTreeSet;

use inbox_core::popup::{Entry, Popup, Snapshot};

use crate::store::Store;

/// What the popup has shown, so a read takes those rows whole.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    /// Groups listed or open.
    pub groups: BTreeSet<i64>,
    /// ⚠ answers listed or open.
    pub failed: BTreeSet<i64>,
    /// Alerts listed or open.
    pub alerts: BTreeSet<i64>,
}

impl Shown {
    /// What `popup` shows (nothing before it opened).
    pub fn of(popup: Option<&Popup>) -> Shown {
        let mut shown = Shown::default();
        let Some(p) = popup else { return shown };
        for e in p.entries().iter().chain(p.entry()) {
            match e {
                Entry::Group(g) => shown.groups.insert(*g),
                Entry::Failed(q) => shown.failed.insert(*q),
                Entry::Alert(a) => shown.alerts.insert(*a),
                _ => false,
            };
        }
        shown
    }
}

/// Reads the database for the model. `Err`: the database failed (busy,
/// locked); the caller keeps the copy it has.
pub fn read(store: &Store, shown: &Shown) -> rusqlite::Result<Snapshot> {
    // one read transaction: every list from the same moment
    store.db().execute_batch("BEGIN DEFERRED")?;
    let snap = read_now(store, shown);
    let end = if snap.is_ok() { "COMMIT" } else { "ROLLBACK" };
    let ended = store.db().execute_batch(end);
    let snap = snap?;
    ended?;
    Ok(snap)
}

fn read_now(store: &Store, shown: &Shown) -> rusqlite::Result<Snapshot> {
    let open = store.list(false)?;
    let failed = store.failed_deliveries()?;
    let mut rows = Vec::new();
    rows.extend(open.iter().cloned());
    rows.extend(failed.iter().cloned());
    for &g in &shown.groups {
        rows.extend(store.group(g, true)?);
    }
    for &q in &shown.failed {
        rows.extend(store.get(q)?);
    }
    rows.sort_by_key(|q| q.id);
    rows.dedup_by_key(|q| q.id);

    let open_alerts = store.open_alerts()?;
    let mut alerts = open_alerts.clone();
    for &a in &shown.alerts {
        if !alerts.iter().any(|x| x.id == a) {
            alerts.extend(store.alert(a)?);
        }
    }

    let mut snap = Snapshot::default();
    snap.questions = rows.iter().map(convert::question).collect();
    snap.open = open.iter().map(|q| q.id).collect();
    snap.open.sort_unstable();
    snap.failed = failed.iter().map(|q| q.id).collect();
    snap.alerts = alerts.iter().map(convert::alert).collect();
    snap.open_alerts = open_alerts.iter().map(|a| a.id).collect();
    Ok(snap)
}

#[cfg(test)]
mod tests;
