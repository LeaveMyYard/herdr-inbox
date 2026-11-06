//! Raising an agent's alert (Python `raise_alert`): from a herdr pane, or from
//! a core terminal, which has no pane, no herdr server and no marks.

use rusqlite::types::Value;
use rusqlite::{OptionalExtension, params, params_from_iter};

use super::agent::Asker;
use super::{Effect, MAX_MESSAGE, MAX_TITLE, SEVERITIES};
use crate::clock::now;
use crate::store::Store;
use crate::store::family::Continuations;
use crate::store::hosts::{CORE_SERVER, CoreSeat};
use crate::text::{py_isspace, squash};

/// What raising an alert did (Python's `status`: `open`, `already_open`,
/// `error`, or the unavailable receipt).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RaiseOutcome {
    /// Filed as this id; then these effects.
    Raised(i64, Vec<Effect>),
    /// The asker's open alert with this title.
    AlreadyOpen(i64),
    /// Nothing left of the title.
    NoTitle,
    /// The asker has no pane and no proved terminal.
    Unavailable,
}

/// Where an alert is raised from.
#[derive(Clone, Copy)]
enum Seat<'a> {
    /// A herdr pane, on this herdr server.
    Pane(&'a str, &'a str),
    /// A core terminal.
    Core(&'a CoreSeat),
}

impl Store {
    /// An agent raises an alert: one per open title per asker. `server` is
    /// this herdr server's id.
    #[allow(clippy::too_many_arguments)]
    pub fn raise_alert(
        &self,
        asker: Asker<'_>,
        title: &str,
        message: Option<&str>,
        severity: &str,
        asked_by: Option<&str>,
        server: &str,
        files: &impl Continuations,
    ) -> rusqlite::Result<RaiseOutcome> {
        let Some(pane) = asker.owner.pane_id.filter(|p| !p.is_empty()) else {
            return Ok(RaiseOutcome::Unavailable);
        };
        let seat = Seat::Pane(pane, server);
        self.raise(asker, title, message, severity, asked_by, seat, files)
    }

    /// [`Store::raise_alert`] for an agent proved to run in core terminal
    /// `seat`: no pane, no herdr server, and nothing done to herdr afterwards.
    #[allow(clippy::too_many_arguments)]
    pub fn raise_core_alert(
        &self,
        asker: Asker<'_>,
        seat: &CoreSeat,
        title: &str,
        message: Option<&str>,
        severity: &str,
        asked_by: Option<&str>,
        files: &impl Continuations,
    ) -> rusqlite::Result<RaiseOutcome> {
        let seat = Seat::Core(seat);
        self.raise(asker, title, message, severity, asked_by, seat, files)
    }

    #[allow(clippy::too_many_arguments)]
    fn raise(
        &self,
        asker: Asker<'_>,
        title: &str,
        message: Option<&str>,
        severity: &str,
        asked_by: Option<&str>,
        seat: Seat<'_>,
        files: &impl Continuations,
    ) -> rusqlite::Result<RaiseOutcome> {
        let (pane, server) = match seat {
            Seat::Pane(pane, server) => (Some(pane), server),
            Seat::Core(_) => (None, CORE_SERVER),
        };
        let severity = if SEVERITIES.contains(&severity) {
            severity
        } else {
            "warning"
        };
        let title: String = squash(title).chars().take(MAX_TITLE).collect();
        if title.is_empty() {
            return Ok(RaiseOutcome::NoTitle);
        }
        let message = message
            .map(|m| {
                m.trim_matches(py_isspace)
                    .chars()
                    .take(MAX_MESSAGE)
                    .collect::<String>()
            })
            .filter(|m| !m.is_empty());
        let (cond, args) = self.owner_clause(asker.owner, files)?;
        let filed = self.immediate(|db| {
            let mut dup_args = vec![Value::Text(title.clone())];
            dup_args.extend(args);
            let dup: Option<i64> = db
                .query_row(
                    &format!("SELECT id FROM alerts WHERE status = 'open' AND kind = 'agent' AND title = ? AND {cond}"),
                    params_from_iter(dup_args),
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = dup {
                return Ok(Err(id));
            }
            db.execute(
                "INSERT INTO alerts (created_at, kind, severity, title, message, pane_id, tab_id, tab_label, session_id, \
                 asked_by, herdr_server) VALUES (?, 'agent', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    now(),
                    severity,
                    title,
                    message,
                    pane,
                    asker.tab_id,
                    asker.tab_label,
                    asker.owner.session_id,
                    asked_by,
                    server
                ],
            )?;
            let id = db.last_insert_rowid();
            if let Seat::Core(seat) = seat {
                self.set_alert_host(id, seat)?;
            }
            Ok(Ok(id))
        })?;
        Ok(match (filed, pane) {
            (Err(id), _) => RaiseOutcome::AlreadyOpen(id),
            (Ok(id), None) => RaiseOutcome::Raised(id, Vec::new()),
            (Ok(id), Some(pane)) => {
                let place = asker.tab_label.filter(|l| !l.is_empty()).unwrap_or(pane);
                RaiseOutcome::Raised(
                    id,
                    vec![
                        Effect::Notify {
                            title: format!("A{id} · {severity} · {place}"),
                            body: title,
                        },
                        Effect::Markers(Some(pane.into())),
                    ],
                )
            }
        })
    }
}
