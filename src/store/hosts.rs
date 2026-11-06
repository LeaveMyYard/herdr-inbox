//! Where an agent outside a herdr pane runs: the `question_host` and
//! `alert_host` side tables (plan herdr-app-inbox §3.2, B-host).
//!
//! A core row has `pane_id` NULL, so no reader can take `t3` for a herdr pane,
//! and [`CORE_SERVER`] as its `herdr_server`, so no herdr server's worker takes
//! it for its own: `served`, `reconcile_failed`, `revive_moved` and
//! `hook_unverified` all skip a row of another server, in both languages.
//! A pane row has no side-table row at all.

use rusqlite::{OptionalExtension, params};

use super::Store;

/// `host` of a core row, as in Panorama's feed and the app feed's `asker.host`.
pub const CORE: &str = "herdr-core";

/// `herdr_server` of a core row. A herdr server's id is its socket's path,
/// which this is not, so it is never any worker's own server.
pub const CORE_SERVER: &str = CORE;

/// A terminal of herdr-core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreSeat {
    /// Its address, such as `t3`.
    pub terminal: String,
    /// The machine it runs on; `None` is this one.
    pub machine: Option<String>,
}

impl Store {
    /// Records that question `qid` was asked from `seat`.
    pub fn set_question_host(&self, qid: i64, seat: &CoreSeat) -> rusqlite::Result<()> {
        self.db().execute(
            "INSERT INTO question_host (id, host, terminal, machine) VALUES (?, ?, ?, ?)",
            params![qid, CORE, seat.terminal, seat.machine],
        )?;
        Ok(())
    }

    /// Where question `qid` was asked, if in a core terminal.
    pub fn core_seat(&self, qid: i64) -> rusqlite::Result<Option<CoreSeat>> {
        self.db()
            .query_row(
                "SELECT terminal, machine FROM question_host WHERE id = ? AND host = ? \
                 AND terminal IS NOT NULL",
                params![qid, CORE],
                |r| {
                    Ok(CoreSeat {
                        terminal: r.get(0)?,
                        machine: r.get(1)?,
                    })
                },
            )
            .optional()
    }

    /// Records that alert `aid` was raised from `seat`.
    pub fn set_alert_host(&self, aid: i64, seat: &CoreSeat) -> rusqlite::Result<()> {
        self.db().execute(
            "INSERT INTO alert_host (id, host, terminal, machine) VALUES (?, ?, ?, ?)",
            params![aid, CORE, seat.terminal, seat.machine],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
