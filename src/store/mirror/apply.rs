//! A validated export written into the mirror (Python `remote.pull`'s
//! transaction): one transaction, and the notices for what is new.

use std::time::SystemTime;

use rusqlite::types::Value as Sql;
use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value};

use super::pull::value_str;
use super::{Machine, Notice, first_line, iso};
use crate::pyjson::dumps;
use crate::remote::validate::{Export, py_int};
use crate::store::Store;

impl Store {
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
