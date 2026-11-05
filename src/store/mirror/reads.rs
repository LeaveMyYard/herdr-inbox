//! What the popup reads (the local mirror only, never the network; Python
//! `open_alerts`, `alert_by_key`, `open_groups`, `group_by_key`, `_busy_ids`).

use std::time::{SystemTime, UNIX_EPOCH};

use inbox_core::popup_view::unix_seconds;
use serde_json::{Map, Value};

use super::pull::value_str;
use super::{MACHINE_COLUMNS, Machine};
use crate::clock::local_hhmm;
use crate::pyjson;
use crate::remote::validate::py_int;
use crate::store::Store;

/// The key of a mirrored group (Python `group_key`).
pub fn group_key(machine_id: &str, db_id: &str, group: &Value) -> String {
    format!("{machine_id}|{db_id}|{}", value_str(group))
}

fn json_map(text: &str) -> rusqlite::Result<Map<String, Value>> {
    match pyjson::loads(text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Ok(Map::new()),
        Err(e) => Err(rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(e),
        )),
    }
}

impl Machine {
    /// `offline since 10:42 (3 h)` when the last pull failed (Python
    /// `offline_note`), `not reached yet` if none ever worked.
    pub fn offline_note(&self, now: SystemTime) -> Option<String> {
        self.last_error.as_ref().filter(|e| !e.is_empty())?;
        let since = self.last_ok_at.as_deref().filter(|s| !s.is_empty());
        let Some(when) = since.and_then(unix_seconds) else {
            return Some("not reached yet".to_string());
        };
        let unix = now
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        let hours = (unix - when as f64) / 3600.0;
        let age = if hours >= 1.0 {
            format!("{hours:.0} h")
        } else {
            format!("{:.0} min", hours * 60.0)
        };
        Some(format!("offline since {} ({age})", local_hhmm(when)))
    }
}

impl Store {
    /// Every mirrored machine (Python `SELECT * … ORDER BY label`).
    pub fn machines_by_label(&self) -> rusqlite::Result<Vec<Machine>> {
        let mut stmt = self.db().prepare(&format!(
            "SELECT {MACHINE_COLUMNS} FROM remote_machines ORDER BY label"
        ))?;
        stmt.query_map([], Machine::from_row)?.collect()
    }

    fn mirrored(&self, sql: &str, machine_id: &str) -> rusqlite::Result<Vec<Map<String, Value>>> {
        let mut stmt = self.db().prepare(sql)?;
        let rows: Vec<(String, String)> = stmt
            .query_map([machine_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(db_id, json)| {
                let mut data = json_map(&json)?;
                data.insert("_db_id".into(), Value::String(db_id));
                Ok(data)
            })
            .collect()
    }

    /// (machine, remote id, db id, qkey) of every mirrored question, open or
    /// not: what an action queued now is checked against.
    pub fn mirror_identities(&self) -> rusqlite::Result<Vec<(String, i64, String, String)>> {
        let mut stmt = self.db().prepare(
            "SELECT machine_id, remote_id, db_id, qkey FROM remote_questions ORDER BY machine_id, remote_id",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect()
    }

    /// (machine, alert data with `_db_id`) for every open mirrored alert.
    pub fn mirror_open_alerts(&self) -> rusqlite::Result<Vec<(Machine, Map<String, Value>)>> {
        let mut out = Vec::new();
        for m in self.machines_by_label()? {
            let rows = self.mirrored(
                "SELECT db_id, data_json FROM remote_alerts WHERE machine_id = ? AND status = 'open' ORDER BY alert_id",
                &m.machine_id,
            )?;
            out.extend(rows.into_iter().map(|a| (m.clone(), a)));
        }
        Ok(out)
    }

    /// The open mirrored alert of key `<machine_id>|<alert id>`.
    pub fn mirror_alert_by_key(
        &self,
        key: &str,
    ) -> rusqlite::Result<Option<(Machine, Map<String, Value>)>> {
        let (mid, aid) = key.split_once('|').unwrap_or((key, ""));
        Ok(self.mirror_open_alerts()?.into_iter().find(|(m, a)| {
            m.machine_id == mid && a.get("id").map(value_str).as_deref() == Some(aid)
        }))
    }

    /// Remote questions with an answer or dismissal queued or done: not answerable again.
    fn busy_ids(&self, machine_id: &str) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self.db().prepare(
            "SELECT remote_id FROM remote_outbox WHERE machine_id = ? AND action IN ('answer', 'dismiss') \
             AND state IN ('queued', 'done')",
        )?;
        stmt.query_map([machine_id], |r| r.get(0))?.collect()
    }

    /// This very question (`db_id`, `qkey`) has an answer or a dismissal queued
    /// or done: the `busy_ids` rule, for a write. A later question under
    /// the same number does not count.
    pub fn action_queued(
        &self,
        machine_id: &str,
        remote_id: i64,
        db_id: &str,
        qkey: &str,
    ) -> rusqlite::Result<bool> {
        self.db().query_row(
            "SELECT EXISTS (SELECT 1 FROM remote_outbox WHERE machine_id = ? AND remote_id = ? AND db_id = ? \
             AND qkey = ? AND action IN ('answer', 'dismiss') AND state IN ('queued', 'done'))",
            rusqlite::params![machine_id, remote_id, db_id, qkey],
            |r| r.get(0),
        )
    }

    /// (group key, machine, open questions' data with `_db_id`) per open
    /// mirrored group, questions already answered here left out.
    #[allow(clippy::type_complexity)]
    pub fn mirror_open_groups(
        &self,
    ) -> rusqlite::Result<Vec<(String, Machine, Vec<Map<String, Value>>)>> {
        let mut out = Vec::new();
        for m in self.machines_by_label()? {
            let busy = self.busy_ids(&m.machine_id)?;
            let mut groups: Vec<((String, Value), Vec<Map<String, Value>>)> = Vec::new();
            let rows = self.mirrored(
                "SELECT db_id, data_json FROM remote_questions WHERE machine_id = ? AND status = 'open' ORDER BY remote_id",
                &m.machine_id,
            )?;
            for data in rows {
                if data
                    .get("id")
                    .and_then(py_int)
                    .is_some_and(|id| busy.contains(&id))
                {
                    continue;
                }
                let key = (
                    data["_db_id"].as_str().unwrap_or_default().to_string(),
                    data.get("group").cloned().unwrap_or(Value::Null),
                );
                match groups
                    .iter_mut()
                    .find(|(k, _)| k.0 == key.0 && py_eq(&k.1, &key.1))
                {
                    Some((_, qs)) => qs.push(data),
                    None => groups.push((key, vec![data])),
                }
            }
            for ((db_id, group), qs) in groups {
                out.push((group_key(&m.machine_id, &db_id, &group), m.clone(), qs));
            }
        }
        Ok(out)
    }

    /// The open mirrored group of `key`.
    #[allow(clippy::type_complexity)]
    pub fn mirror_group_by_key(
        &self,
        key: &str,
    ) -> rusqlite::Result<Option<(Machine, Vec<Map<String, Value>>)>> {
        Ok(self
            .mirror_open_groups()?
            .into_iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, m, qs)| (m, qs)))
    }
}

/// Python's `==` between two dict keys (`True == 1`).
fn py_eq(a: &Value, b: &Value) -> bool {
    match (py_int(a), py_int(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}
