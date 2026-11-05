//! How long each pending answer has waited (Python `service._busy_for`):
//! the renderer's "agent busy — will send in N s". Read with the snapshot
//! (the database, on the popup's thread); turned into seconds at each frame.

use std::collections::HashMap;

use inbox_core::popup::Snapshot;
use inbox_core::popup_view::unix_seconds;

use crate::store::Store;
use crate::store::questions::PENDING;

/// Since when (Unix seconds) each pending answer of the snapshot waits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BusySince(HashMap<i64, i64>);

impl BusySince {
    /// Reads `pending_since` for every pending answer in `snap`.
    pub fn read(store: &Store, snap: &Snapshot) -> rusqlite::Result<BusySince> {
        let mut since = HashMap::new();
        for q in &snap.questions {
            let pending = q
                .delivery_error
                .as_deref()
                .is_some_and(|e| e.starts_with(PENDING));
            if !pending {
                continue;
            }
            if let Some(t) = store.pending_since(q.id)?.as_deref().and_then(unix_seconds) {
                since.insert(q.id, t);
            }
        }
        Ok(BusySince(since))
    }

    /// Seconds waited at `wall` (Unix seconds), by question (0 for one not
    /// pending, as Python's; negative if the clocks disagree, as there).
    pub fn at(&self, wall: f64) -> HashMap<i64, f64> {
        self.0.iter().map(|(&q, &t)| (q, wall - t as f64)).collect()
    }
}
