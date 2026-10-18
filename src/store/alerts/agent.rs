//! Alerts an agent raises (Python `raise_alert`, `get`, `open_alerts`,
//! `acknowledge`, `withdraw`, `acknowledged_unreported`, `mine`).

use rusqlite::types::Value;
use rusqlite::{OptionalExtension, params, params_from_iter};

use super::{Alert, COLUMNS, Effect, MAX_MESSAGE, MAX_TITLE, SEVERITIES};
use crate::clock::now;
use crate::store::Store;
use crate::store::family::{Continuations, family};
use crate::store::questions::Owner;
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
    /// The asker has no pane.
    Unavailable,
}

/// What withdrawing did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WithdrawOutcome {
    /// Withdrawn; then these effects.
    Withdrawn(Vec<Effect>),
    /// No agent alert of the asker's with that id.
    NotYours,
    /// Not open; its status.
    NotOpen(String),
}

/// The asker of an alert (Python `service.Caller`'s fields used here).
#[derive(Clone, Copy, Debug)]
pub struct Asker<'a> {
    /// Session and pane.
    pub owner: Owner<'a>,
    /// Its tab.
    pub tab_id: Option<&'a str>,
    /// Its tab's label.
    pub tab_label: Option<&'a str>,
}

impl Store {
    fn alerts_where(&self, sql: &str, args: Vec<Value>) -> rusqlite::Result<Vec<Alert>> {
        let mut stmt = self.db().prepare(sql)?;
        let rows = stmt.query_map(params_from_iter(args), Alert::from_row)?;
        rows.collect()
    }

    /// The alert with id `aid`.
    pub fn alert(&self, aid: i64) -> rusqlite::Result<Option<Alert>> {
        self.db()
            .query_row(
                &format!("SELECT {COLUMNS} FROM alerts WHERE id = ?"),
                [aid],
                Alert::from_row,
            )
            .optional()
    }

    /// Open alerts, urgent first, then warning, then the rest, oldest first.
    pub fn open_alerts(&self) -> rusqlite::Result<Vec<Alert>> {
        self.alerts_where(
            &format!(
                "SELECT {COLUMNS} FROM alerts WHERE status = 'open' ORDER BY \
                 CASE severity WHEN 'urgent' THEN 0 WHEN 'warning' THEN 1 ELSE 2 END, id"
            ),
            Vec::new(),
        )
    }

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
            Ok(Ok(db.last_insert_rowid()))
        })?;
        Ok(match filed {
            Err(id) => RaiseOutcome::AlreadyOpen(id),
            Ok(id) => {
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

    /// Closes open alert `aid` as `status` (`acked` stamps `acked_at`, the
    /// rest `closed_at`): the alert as it was, `None` if it was not open.
    pub(super) fn close_alert(&self, aid: i64, status: &str) -> rusqlite::Result<Option<Alert>> {
        let Some(alert) = self.alert(aid)?.filter(|a| a.status == "open") else {
            return Ok(None);
        };
        let column = if status == "acked" {
            "acked_at"
        } else {
            "closed_at"
        };
        let n = self.db().execute(
            &format!("UPDATE alerts SET status = ?, {column} = ? WHERE id = ? AND status = 'open'"),
            params![status, now(), aid],
        )?;
        Ok((n == 1).then_some(alert))
    }

    /// The operator acknowledged alert `aid`: the effects, `None` if it was not open.
    pub fn acknowledge_alert(&self, aid: i64) -> rusqlite::Result<Option<Vec<Effect>>> {
        Ok(self
            .close_alert(aid, "acked")?
            .map(|a| vec![Effect::Markers(a.pane_id)]))
    }

    fn is_mine(
        &self,
        alert: &Alert,
        owner: Owner<'_>,
        files: &impl Continuations,
    ) -> rusqlite::Result<bool> {
        Ok(match owner.session_id.filter(|s| !s.is_empty()) {
            Some(session) => {
                let fam = family(self.db(), session, files)?;
                alert.session_id.as_ref().is_some_and(|s| fam.contains(s))
            }
            None => alert.session_id.is_none() && alert.pane_id.as_deref() == owner.pane_id,
        })
    }

    /// The asker withdraws its alert `aid`.
    pub fn withdraw_alert(
        &self,
        owner: Owner<'_>,
        aid: i64,
        files: &impl Continuations,
    ) -> rusqlite::Result<WithdrawOutcome> {
        let Some(alert) = self.alert(aid)? else {
            return Ok(WithdrawOutcome::NotYours);
        };
        if alert.kind != "agent" || !self.is_mine(&alert, owner, files)? {
            return Ok(WithdrawOutcome::NotYours);
        }
        Ok(match self.close_alert(aid, "withdrawn")? {
            Some(a) => WithdrawOutcome::Withdrawn(vec![Effect::Markers(a.pane_id)]),
            None => WithdrawOutcome::NotOpen(alert.status),
        })
    }

    /// The asker's agent alerts the operator acknowledged since the last
    /// report (check_answers); each is marked reported.
    pub fn acknowledged_unreported(
        &self,
        owner: Owner<'_>,
        files: &impl Continuations,
    ) -> rusqlite::Result<Vec<Alert>> {
        let (cond, args) = self.owner_clause(owner, files)?;
        let out = self.alerts_where(
            &format!(
                "SELECT {COLUMNS} FROM alerts WHERE kind = 'agent' AND status = 'acked' AND reported_at IS NULL \
                 AND {cond} ORDER BY id"
            ),
            args,
        )?;
        for a in &out {
            self.db().execute(
                "UPDATE alerts SET reported_at = ? WHERE id = ?",
                params![now(), a.id],
            )?;
        }
        Ok(out)
    }

    /// The asker's agent alerts, every status.
    pub fn my_alerts(
        &self,
        owner: Owner<'_>,
        files: &impl Continuations,
    ) -> rusqlite::Result<Vec<Alert>> {
        let (cond, args) = self.owner_clause(owner, files)?;
        self.alerts_where(
            &format!("SELECT {COLUMNS} FROM alerts WHERE kind = 'agent' AND {cond} ORDER BY id"),
            args,
        )
    }
}
