//! One pull of a remote into the mirror (Python `remote.pull`, `_record`,
//! `_fail`), less its ssh: the caller passes the paths probe and the call.

use std::time::{Duration, SystemTime};

use rusqlite::types::Value as Sql;
use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value};

use super::{MACHINE_COLUMNS, Machine, Notice, first_line, iso};
use crate::pyjson::dumps;
use crate::remote::validate::{Export, ExportError, py_int, validate_export};
use crate::store::Store;

/// Every this many pulls is a full snapshot (Python `FULL_EVERY`).
pub const FULL_EVERY: i64 = 60;

/// What the remote's `inbox export` call returned.
#[derive(Clone, Debug, Default)]
pub struct CallResult {
    /// Its exit code.
    pub code: i32,
    /// Its stdout.
    pub stdout: String,
    /// Its stderr.
    pub stderr: String,
}

/// Python's `str()` of an exported id (an int, or a bool Python took for one).
pub(super) fn value_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        other => crate::pyjson::dumps(other),
    }
}

impl Store {
    /// Records `machine` (id, label, target) and returns its row (Python `_record`).
    pub fn record_machine(
        &self,
        id: &str,
        label: Option<&str>,
        target: Option<&str>,
    ) -> rusqlite::Result<Machine> {
        self.immediate(|db| {
            db.execute(
                "INSERT INTO remote_machines (machine_id, label, target) VALUES (?, ?, ?) \
                 ON CONFLICT(machine_id) DO UPDATE SET label = excluded.label, target = excluded.target",
                params![id, label, target],
            )?;
            db.query_row(&format!("SELECT {MACHINE_COLUMNS} FROM remote_machines WHERE machine_id = ?"), [id], Machine::from_row)
        })
    }

    /// A failed pull: the error, and a backoff of 5, 10, 20, 40, then 60 s.
    pub fn fail_pull(&self, rec: &Machine, error: &str, now: SystemTime) -> rusqlite::Result<()> {
        let delay = (5u64 << rec.failures.clamp(0, 4)).min(60);
        let error: String = error.chars().take(300).collect();
        self.db().execute(
            "UPDATE remote_machines SET last_error = ?, last_try_at = ?, backoff_until = ?, \
             failures = failures + 1 WHERE machine_id = ?",
            params![
                error,
                iso(now),
                iso(now + Duration::from_secs(delay)),
                rec.machine_id
            ],
        )?;
        Ok(())
    }

    /// One pull of `id` into the mirror: `true` if it worked, and the
    /// notices to send. `ssh` runs the remote's probe and `herdr plugin list`
    /// while a path is not cached ([`Store::machine_paths`]); `call` runs
    /// `inbox export` there with the cursor (`db_id`, `seq`) and whether to
    /// ask for a full snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn pull(
        &self,
        id: &str,
        label: Option<&str>,
        target: Option<&str>,
        now: SystemTime,
        probe: impl FnOnce(&str) -> String,
        plugin_list: impl FnOnce(&str, &str) -> String,
        call: impl FnOnce(&Machine, Option<(String, Option<i64>)>, bool) -> CallResult,
    ) -> rusqlite::Result<(bool, Vec<Notice>)> {
        let rec = self.record_machine(id, label, target)?;
        if rec
            .backoff_until
            .as_deref()
            .is_some_and(|b| b > iso(now).as_str())
        {
            return Ok((false, Vec::new()));
        }
        let rec = match self.machine_paths(&rec, probe, plugin_list)? {
            Ok(r) => r,
            Err(e) => {
                self.fail_pull(&rec, &e, now)?;
                return Ok((false, Vec::new()));
            }
        };
        let cursor = rec
            .db_id
            .clone()
            .filter(|d| !d.is_empty())
            .map(|d| (d, rec.cursor_seq));
        let full = cursor.is_none() || rec.pulls.rem_euclid(FULL_EVERY) == 0;
        let result = call(&rec, cursor, full);
        let failure = match result.code {
            2 => {
                Some("upgrade needed: its inbox has no `export` (inbox remote upgrade)".to_string())
            }
            0 => None,
            code => {
                let tail: Vec<char> = result
                    .stderr
                    .trim_matches(crate::text::py_isspace)
                    .chars()
                    .collect();
                let tail: String = tail[tail.len().saturating_sub(300)..].iter().collect();
                Some(if tail.is_empty() {
                    format!("ssh exit {code}")
                } else {
                    tail
                })
            }
        };
        if let Some(error) = failure {
            self.fail_pull(&rec, &error, now)?;
            return Ok((false, Vec::new()));
        }
        let data = match validate_export(&result.stdout) {
            Ok(d) => d,
            Err(ExportError::Format(m)) => {
                self.fail_pull(&rec, &format!("upgrade needed: {m}"), now)?;
                return Ok((false, Vec::new()));
            }
            Err(ExportError::Bad(m)) => {
                self.fail_pull(&rec, &m, now)?;
                return Ok((false, Vec::new()));
            }
        };
        let mut notices = self.apply_pull(&rec, &data, now)?;
        notices.extend(self.notify_problems(&rec.machine_id, rec.name(), now)?);
        Ok((true, notices))
    }

    /// Writes a validated export into the mirror, in one transaction: the
    /// notices for new open questions, new open alerts and a reset.
    pub fn apply_pull(
        &self,
        rec: &Machine,
        data: &Export,
        now: SystemTime,
    ) -> rusqlite::Result<Vec<Notice>> {
        let label = rec.name().to_string();
        let mid = rec.machine_id.as_str();
        let at = iso(now);
        self.immediate(|db| {
            let mut new = Vec::new();
            if rec.db_id.as_deref().is_some_and(|d| !d.is_empty() && d != data.db_id) {
                db.execute("DELETE FROM remote_questions WHERE machine_id = ?", [mid])?;
                db.execute("DELETE FROM remote_alerts WHERE machine_id = ?", [mid])?;
                let failed = db.execute(
                    "UPDATE remote_outbox SET state = 'failed', result = 'reset' WHERE machine_id = ? AND state = 'queued'",
                    [mid],
                )?;
                if failed > 0 {
                    new.push(Notice {
                        title: format!("{label}'s inbox was reset"),
                        body: first_line(&format!("{failed} queued answer(s) were not sent — answer again")),
                    });
                }
            }
            for row in &data.questions {
                let rid = py_int(&row["id"]).unwrap_or_default();
                let seen = db
                    .query_row("SELECT 1 FROM remote_questions WHERE machine_id = ? AND remote_id = ?", params![mid, rid], |_| Ok(()))
                    .optional()?
                    .is_some();
                let status = row["status"].as_str().unwrap_or_default();
                db.execute(
                    "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(machine_id, remote_id) DO UPDATE SET \
                     db_id = excluded.db_id, qkey = excluded.qkey, status = excluded.status, data_json = excluded.data_json",
                    params![mid, data.db_id, rid, row["qkey"].as_str(), status, dumps(&Value::Object(row.clone())), at],
                )?;
                if !seen && status == "open" {
                    let place = [&row["tab_label"], &row["pane_id"]]
                        .into_iter()
                        .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
                        .unwrap_or("");
                    new.push(Notice {
                        title: format!("{label} · Q{} · {place}", value_str(&row["id"])),
                        body: first_line(row["question"].as_str().unwrap_or_default()),
                    });
                }
            }
            for rid in &data.gone {
                db.execute("DELETE FROM remote_questions WHERE machine_id = ? AND remote_id = ?", params![mid, rid])?;
            }
            if data.full {
                let keep: std::collections::HashSet<i64> =
                    data.ids.iter().copied().chain(data.questions.iter().filter_map(|r| py_int(&r["id"]))).collect();
                let have: Vec<i64> = {
                    let mut stmt = db.prepare("SELECT remote_id FROM remote_questions WHERE machine_id = ?")?;
                    stmt.query_map([mid], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
                };
                for rid in have.into_iter().filter(|r| !keep.contains(r)) {
                    db.execute("DELETE FROM remote_questions WHERE machine_id = ? AND remote_id = ?", params![mid, rid])?;
                }
            }
            if let Some(alerts) = &data.alerts {
                new.extend(replace_alerts(db, mid, &label, &data.db_id, alerts, &at)?);
            }
            db.execute(
                "UPDATE remote_machines SET db_id = ?, cursor_seq = ?, version = ?, pulls = pulls + 1, last_ok_at = ?, \
                 last_error = NULL, last_try_at = ?, backoff_until = NULL, failures = 0 WHERE machine_id = ?",
                params![data.db_id, data.seq, data.version, at, at, mid],
            )?;
            Ok(new)
        })
    }
}

/// The remote's alerts replace the mirrored ones; one queued here for an
/// acknowledgement stays acknowledged.
fn replace_alerts(
    db: &rusqlite::Connection,
    mid: &str,
    label: &str,
    db_id: &str,
    alerts: &[Map<String, Value>],
    at: &str,
) -> rusqlite::Result<Vec<Notice>> {
    let ids = |sql: &str| -> rusqlite::Result<Vec<i64>> {
        let mut stmt = db.prepare(sql)?;
        stmt.query_map([mid], |r| r.get(0))?.collect()
    };
    let known = ids("SELECT alert_id FROM remote_alerts WHERE machine_id = ?")?;
    let acking = ids(
        "SELECT remote_id FROM remote_outbox WHERE machine_id = ? AND action = 'ack_alert' AND state = 'queued'",
    )?;
    db.execute("DELETE FROM remote_alerts WHERE machine_id = ?", [mid])?;
    let mut new = Vec::new();
    for a in alerts {
        let id = py_int(&a["id"]).unwrap_or_default();
        let status = a["status"].as_str().unwrap_or_default();
        let shown = if acking.contains(&id) && status == "open" {
            "acked"
        } else {
            status
        };
        db.execute(
            "INSERT INTO remote_alerts (machine_id, db_id, alert_id, akey, status, data_json, first_seen_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![mid, db_id, Sql::Integer(id), a["akey"].as_str(), shown, dumps(&Value::Object(a.clone())), at],
        )?;
        if !known.contains(&id) && status == "open" {
            let tab = a["tab_label"]
                .as_str()
                .filter(|t| !t.is_empty())
                .unwrap_or("");
            new.push(Notice {
                title: format!(
                    "{label} · A{} · {} · {tab}",
                    value_str(&a["id"]),
                    a["severity"].as_str().unwrap_or_default()
                ),
                body: first_line(a["title"].as_str().unwrap_or_default()),
            });
        }
    }
    Ok(new)
}
