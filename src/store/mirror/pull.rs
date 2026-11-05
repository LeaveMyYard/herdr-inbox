//! One pull of a remote into the mirror (Python `remote.pull`, `_record`,
//! `_fail`), less its ssh: the caller passes the paths probe and the call.

use std::time::{Duration, SystemTime};

use rusqlite::params;
use serde_json::{Value, json};

use super::{Clock, MACHINE_COLUMNS, Machine, Notice, iso};
use crate::pyjson::dumps;
use crate::remote::command::{FixedCommand, InboxSub, SafeTarget, Ssh};
use crate::remote::validate::{ExportError, validate_export};
use crate::store::{Partial, Store};

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
    /// notices to send (of what it committed before a database error too; a
    /// pull with an error did not work, as Python's raises before its flush).
    /// `ssh(target, command, stdin)` runs a fixed command on the remote (the
    /// paths probe, `herdr plugin list`, `inbox export` with its request on
    /// stdin); the commands are built and checked here. `clock` is read as
    /// Python reads it: a failure's backoff counts from the failure, not
    /// from the start of the call that failed.
    pub fn pull(
        &self,
        id: &str,
        label: Option<&str>,
        target: Option<&str>,
        clock: Clock<'_>,
        ssh: Ssh<'_>,
    ) -> Partial<(bool, Vec<Notice>)> {
        let mut notices = Vec::new();
        match self.pull_into(id, label, target, clock, ssh, &mut notices) {
            Ok(ok) => Partial::of((ok, notices), Ok(())),
            Err(e) => Partial::of((false, notices), Err(e)),
        }
    }

    fn pull_into(
        &self,
        id: &str,
        label: Option<&str>,
        target: Option<&str>,
        clock: Clock<'_>,
        ssh: Ssh<'_>,
        notices: &mut Vec<Notice>,
    ) -> rusqlite::Result<bool> {
        let rec = self.record_machine(id, label, target)?;
        if rec
            .backoff_until
            .as_deref()
            .is_some_and(|b| b > iso(clock()).as_str())
        {
            return Ok(false);
        }
        let rec = match self.machine_paths(&rec, ssh)? {
            Ok(r) => r,
            Err(e) => {
                self.fail_pull(&rec, &e, clock())?;
                return Ok(false);
            }
        };
        let cursor = rec
            .db_id
            .clone()
            .filter(|d| !d.is_empty())
            .map(|d| (d, rec.cursor_seq));
        let full = cursor.is_none() || rec.pulls.rem_euclid(FULL_EVERY) == 0;
        let request = json!({"cursor": cursor.map(|(db, seq)| json!({"db_id": db, "seq": seq})), "full": full});
        // Python call(): the target, then the cached paths, before ssh runs
        let checked = SafeTarget::for_call(rec.target.as_deref())
            .and_then(|t| Ok((t, FixedCommand::inbox(&rec, InboxSub::Export)?)));
        let result = match checked {
            Ok((target, command)) => ssh(&target, &command, Some(&dumps(&request))),
            Err(e) => {
                self.fail_pull(&rec, &e, clock())?;
                return Ok(false);
            }
        };
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
            self.fail_pull(&rec, &error, clock())?;
            return Ok(false);
        }
        let data = match validate_export(&result.stdout) {
            Ok(d) => d,
            Err(ExportError::Format(m)) => {
                self.fail_pull(&rec, &format!("upgrade needed: {m}"), clock())?;
                return Ok(false);
            }
            Err(ExportError::Bad(m)) => {
                self.fail_pull(&rec, &m, clock())?;
                return Ok(false);
            }
        };
        notices.extend(self.apply_pull(&rec, &data, clock())?);
        let problems = self.notify_problems(&rec.machine_id, rec.name(), clock());
        notices.extend(problems.value);
        problems.error.map_or(Ok(true), Err)
    }
}
