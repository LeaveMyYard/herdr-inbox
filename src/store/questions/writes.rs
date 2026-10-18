//! Question writes (Python `Store.insert`, `set_group_title`, `set_draft`,
//! `set_answer`, `set_status`, `bump_repeat`, `acknowledge`, `redeliver`,
//! `hide_failed`), statement for statement.

use rusqlite::params;
use serde_json::Value;

use super::PENDING;
use crate::clock::now;
use crate::pyjson::dumps;
use crate::store::Store;
use crate::store::norm::{clean_title, normalize};
use crate::store::question::{QOption, Question, labels_json, options_json};

/// What an agent asks (Python `Store.insert`'s arguments).
#[derive(Clone, Debug, Default)]
pub struct NewQuestion<'a> {
    /// The question.
    pub question: &'a str,
    /// Its options, already normalized.
    pub options: Option<Vec<QOption>>,
    /// Context.
    pub context: Option<&'a str>,
    /// The agent's role.
    pub asked_by: Option<&'a str>,
    /// Where it was asked from.
    pub pane_id: Option<&'a str>,
    /// That pane's tab.
    pub tab_id: Option<&'a str>,
    /// The tab's label.
    pub tab_label: Option<&'a str>,
    /// The asking session.
    pub session_id: Option<&'a str>,
    /// The agent's working directory.
    pub cwd: Option<&'a str>,
    /// A short header.
    pub header: Option<&'a str>,
    /// Several options may be picked.
    pub multi_select: bool,
    /// The call's group (`None`: this question starts one).
    pub group_id: Option<i64>,
}

impl Store {
    /// Files a question; returns it as stored.
    pub fn insert(&self, q: &NewQuestion<'_>) -> rusqlite::Result<Question> {
        let options = q.options.as_deref().filter(|o| !o.is_empty());
        self.db().execute(
            "INSERT INTO questions (created_at, pane_id, tab_id, tab_label, session_id, cwd, asked_by, \
             question, options, context, norm) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                now(), q.pane_id, q.tab_id, q.tab_label, q.session_id, q.cwd, q.asked_by, q.question,
                options.map(labels_json), q.context, normalize(q.question)
            ],
        )?;
        let id = self.db().last_insert_rowid();
        self.db().execute(
            "INSERT INTO question_ext (id, header, multi_select, group_id, options_json) VALUES (?, ?, ?, ?, ?)",
            params![id, q.header, i64::from(q.multi_select), q.group_id.filter(|g| *g != 0).unwrap_or(id), options.map(options_json)],
        )?;
        self.get(id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// Titles a group (a title that cleans to nothing is ignored).
    pub fn set_group_title(&self, group_id: i64, title: Option<&str>) -> rusqlite::Result<()> {
        if let Some(title) = clean_title(title) {
            self.db().execute(
                "INSERT OR REPLACE INTO question_groups (group_id, title) VALUES (?, ?)",
                params![group_id, title],
            )?;
        }
        Ok(())
    }

    /// Stores (or with `None` clears) the operator's unsent answer to an open question.
    pub fn set_draft(&self, qid: i64, detail: Option<&Value>) -> rusqlite::Result<bool> {
        let detail = detail.filter(|d| truthy(d));
        let n = self.db().execute(
            "INSERT INTO question_ext (id, group_id, draft_json, drafted_at) \
             SELECT id, id, ?, ? FROM questions WHERE id = ? AND status = 'open' \
             ON CONFLICT(id) DO UPDATE SET draft_json = excluded.draft_json, drafted_at = excluded.drafted_at",
            params![detail.map(dumps), detail.map(|_| now()), qid],
        )?;
        Ok(n == 1)
    }

    /// Answers an open question (`detail`: its parts); `false` when it is not open.
    pub fn set_answer(
        &self,
        qid: i64,
        answer: &str,
        detail: Option<&Value>,
    ) -> rusqlite::Result<bool> {
        let detail = detail.filter(|d| truthy(d));
        self.immediate(|db| {
            let n = db.execute(
                "UPDATE questions SET status = 'answered', answer = ?, answered_at = ?, delivered_at = NULL, \
                 delivery_error = NULL, acknowledged_at = NULL WHERE id = ? AND status = 'open'",
                params![answer, now(), qid],
            )?;
            if n != 1 {
                return Ok(false);
            }
            db.execute(
                "INSERT INTO question_ext (id, group_id, answer_json) VALUES (?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET answer_json = excluded.answer_json, draft_json = NULL, drafted_at = NULL",
                params![qid, qid, detail.map(dumps)],
            )?;
            Ok(true)
        })
    }

    /// Closes an open question (`dismissed`, `withdrawn`); `note` goes into `answer`.
    pub fn set_status(&self, qid: i64, status: &str, note: Option<&str>) -> rusqlite::Result<bool> {
        let n = self.db().execute(
            "UPDATE questions SET status = ?, answer = ?, answered_at = ? WHERE id = ? AND status = 'open'",
            params![status, note, now(), qid],
        )?;
        Ok(n == 1)
    }

    /// The agent asked it again.
    pub fn bump_repeat(&self, qid: i64) -> rusqlite::Result<()> {
        self.db().execute(
            "UPDATE questions SET repeat_count = repeat_count + 1 WHERE id = ?",
            [qid],
        )?;
        Ok(())
    }

    /// The agent has these answers (check_answers): seen, and a stale error on
    /// a seen answer is dropped (a claim in flight is kept).
    pub fn acknowledge(&self, qids: &[i64]) -> rusqlite::Result<()> {
        let mut stmt = self.db().prepare(
            "UPDATE questions SET acknowledged_at = COALESCE(acknowledged_at, ?), \
             delivery_error = CASE WHEN delivery_error LIKE 'sending:%' THEN delivery_error ELSE NULL END WHERE id = ?",
        )?;
        for qid in qids {
            stmt.execute(params![now(), qid])?;
        }
        Ok(())
    }

    /// Makes answered questions pending delivery again (and unseen); returns
    /// the ids that were answered.
    pub fn redeliver(&self, qids: &[i64]) -> rusqlite::Result<Vec<i64>> {
        let mut done = Vec::new();
        for &qid in qids {
            let n = self.db().execute(
                "UPDATE questions SET delivered_at = NULL, acknowledged_at = NULL, delivery_attempts = 0, \
                 delivery_error = ? WHERE id = ? AND status = 'answered'",
                params![format!("{PENDING}redelivery requested"), qid],
            )?;
            if n > 0 {
                self.db().execute(
                    "UPDATE question_ext SET verified_at = NULL, pending_since = NULL, busy_attempts = NULL WHERE id = ?",
                    [qid],
                )?;
                done.push(qid);
            }
        }
        Ok(done)
    }

    /// Stops showing a failed delivery in the popup (check_answers still returns it).
    pub fn hide_failed(&self, qid: i64) -> rusqlite::Result<()> {
        self.db().execute(
            "INSERT INTO question_ext (id, group_id, hidden_at) VALUES (?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET hidden_at = excluded.hidden_at",
            params![qid, qid, now()],
        )?;
        Ok(())
    }
}

/// Python's `if detail` for a dict: `None` and `{}` are no detail.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Object(m) => !m.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::String(s) => !s.is_empty(),
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
    }
}
