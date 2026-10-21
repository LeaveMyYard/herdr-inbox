//! Mirrored answers the remote applied but cannot deliver (Python
//! `_problem`, `problem_rows`, `notify_problems`, `clear_problem`).

use std::time::SystemTime;

use rusqlite::params;
use serde_json::Value;

use super::pull::value_str;
use super::{Notice, first_line, iso};
use crate::pyjson;
use crate::remote::validate::py_int;
use crate::store::Store;

/// The `delivery_error` of a typed answer never seen in the conversation
/// (Python `service.NOT_VERIFIED`).
pub const NOT_VERIFIED: &str =
    "not verified: herdr accepted the text but it never reached the conversation";
/// Unverified tries after which the remote gives up (Python `service.MAX_UNVERIFIED_ATTEMPTS`).
const MAX_UNVERIFIED_ATTEMPTS: i64 = 2;

/// A stuck mirrored answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// Its machine.
    pub machine_id: String,
    /// Its id there.
    pub remote_id: i64,
    /// The machine's label (else its id).
    pub label: String,
    /// Why it is stuck.
    pub why: String,
    /// The mirrored row.
    pub data: Value,
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Why a mirrored answered question is stuck on its machine (only terminal states).
fn problem(data: &Value) -> Option<String> {
    if data.get("status").and_then(Value::as_str) != Some("answered")
        || truthy(data.get("acknowledged_at"))
        || truthy(data.get("verified_at"))
    {
        return None;
    }
    if truthy(data.get("agent_gone")) {
        return Some("agent closed — answer kept for check_answers".into());
    }
    let err = data
        .get("delivery_error")
        .and_then(Value::as_str)
        .unwrap_or("");
    let attempts = data.get("delivery_attempts").and_then(py_int).unwrap_or(0);
    (err.starts_with(NOT_VERIFIED) && attempts >= MAX_UNVERIFIED_ATTEMPTS)
        .then(|| format!("not delivered: {err}"))
}

impl Store {
    /// Mirrored answers the remote applied but cannot deliver, not cleared here.
    pub fn problem_rows(&self) -> rusqlite::Result<Vec<Problem>> {
        let mut stmt = self.db().prepare(
            "SELECT q.machine_id, q.remote_id, q.data_json, m.label FROM remote_questions q \
             LEFT JOIN remote_machines m USING (machine_id) \
             WHERE q.status = 'answered' AND q.cleared_at IS NULL ORDER BY q.remote_id",
        )?;
        let rows: Vec<(String, i64, String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (machine_id, remote_id, json, label) in rows {
            let data = pyjson::loads(&json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            if let Some(why) = problem(&data) {
                let label = label
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| machine_id.clone());
                out.push(Problem {
                    machine_id,
                    remote_id,
                    label,
                    why,
                    data,
                });
            }
        }
        Ok(out)
    }

    /// One notice per stuck answer of `machine_id`, once (a compare-and-set on
    /// `issue_notified_at`).
    pub fn notify_problems(
        &self,
        machine_id: &str,
        label: &str,
        now: SystemTime,
    ) -> rusqlite::Result<Vec<Notice>> {
        let mut out = Vec::new();
        for p in self
            .problem_rows()?
            .into_iter()
            .filter(|p| p.machine_id == machine_id)
        {
            let won = self.db().execute(
                "UPDATE remote_questions SET issue_notified_at = ? WHERE machine_id = ? AND remote_id = ? \
                 AND issue_notified_at IS NULL",
                params![iso(now), machine_id, p.remote_id],
            )?;
            if won > 0 {
                let why: String = p
                    .why
                    .strip_prefix("not delivered: ")
                    .unwrap_or(&p.why)
                    .chars()
                    .take(80)
                    .collect();
                let question = p
                    .data
                    .get("question")
                    .and_then(Value::as_str)
                    .filter(|q| !q.is_empty())
                    .unwrap_or("");
                out.push(Notice {
                    title: format!(
                        "{label}:Q{} not delivered · {why}",
                        value_str(&Value::from(p.remote_id))
                    ),
                    body: first_line(question),
                });
            }
        }
        Ok(out)
    }

    /// The operator dealt with a stuck mirrored answer.
    pub fn clear_problem(
        &self,
        machine_id: &str,
        remote_id: i64,
        now: SystemTime,
    ) -> rusqlite::Result<()> {
        self.db().execute(
            "UPDATE remote_questions SET cleared_at = ? WHERE machine_id = ? AND remote_id = ?",
            params![iso(now), machine_id, remote_id],
        )?;
        Ok(())
    }
}
