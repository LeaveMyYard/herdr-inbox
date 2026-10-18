//! Question reads (Python `Store.get`, `find_duplicate`, `mine`, `list`,
//! `group`, `drafted`, `open_counts`, `known_panes`, `unacknowledged_answers`).

use std::collections::BTreeMap;

use rusqlite::types::Value;
use rusqlite::{OptionalExtension, params_from_iter};

use super::{ANSWER_REUSE, Owner};
use crate::clock::now_minus;
use crate::store::Store;
use crate::store::family::Continuations;
use crate::store::question::{COLUMNS, Question, VIEW};

impl Store {
    /// The question with id `qid`.
    pub fn get(&self, qid: i64) -> rusqlite::Result<Option<Question>> {
        self.db()
            .query_row(
                &format!("SELECT {COLUMNS} FROM {VIEW} WHERE id = ?"),
                [qid],
                Question::from_row,
            )
            .optional()
    }

    fn questions(&self, sql: &str, args: Vec<Value>) -> rusqlite::Result<Vec<Question>> {
        let mut stmt = self.db().prepare(sql)?;
        let rows = stmt.query_map(params_from_iter(args), Question::from_row)?;
        rows.collect()
    }

    /// `owner`'s question with this `norm` that a repeat gets back: an open one
    /// first, else one answered within [`ANSWER_REUSE`], the newest.
    pub fn find_duplicate(
        &self,
        norm: &str,
        owner: Owner<'_>,
        files: &impl Continuations,
    ) -> rusqlite::Result<Option<Question>> {
        let (cond, mut args) = self.owner_clause(owner, files)?;
        args.push(Value::Text(norm.into()));
        args.push(Value::Text(now_minus(ANSWER_REUSE)));
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE {cond} AND norm = ? \
             AND (status = 'open' OR (status = 'answered' AND answered_at >= ?)) \
             ORDER BY status = 'open' DESC, id DESC LIMIT 1"
        );
        Ok(self.questions(&sql, args)?.into_iter().next())
    }

    /// `owner`'s questions, by id: open and answered, or all.
    pub fn mine(
        &self,
        owner: Owner<'_>,
        include_closed: bool,
        files: &impl Continuations,
    ) -> rusqlite::Result<Vec<Question>> {
        let (mut cond, args) = self.owner_clause(owner, files)?;
        if !include_closed {
            cond.push_str(" AND status IN ('open', 'answered')");
        }
        self.questions(
            &format!("SELECT {COLUMNS} FROM {VIEW} WHERE {cond} ORDER BY id"),
            args,
        )
    }

    /// Every open question (or every question), newest first.
    pub fn list(&self, include_closed: bool) -> rusqlite::Result<Vec<Question>> {
        let cond = if include_closed {
            "1"
        } else {
            "status = 'open'"
        };
        self.questions(
            &format!("SELECT {COLUMNS} FROM {VIEW} WHERE {cond} ORDER BY id DESC"),
            Vec::new(),
        )
    }

    /// A group's open questions (or all), by id.
    pub fn group(&self, group_id: i64, include_closed: bool) -> rusqlite::Result<Vec<Question>> {
        let open = if include_closed {
            ""
        } else {
            " AND status = 'open'"
        };
        let sql = format!("SELECT {COLUMNS} FROM {VIEW} WHERE group_id = ?{open} ORDER BY id");
        self.questions(&sql, vec![Value::Integer(group_id)])
    }

    /// Open questions with an unsent draft, by id.
    pub fn drafted(&self) -> rusqlite::Result<Vec<Question>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE status = 'open' AND draft_json IS NOT NULL ORDER BY id"
        );
        self.questions(&sql, Vec::new())
    }

    /// `owner`'s answers the agent has not had yet, by id.
    pub fn unacknowledged_answers(
        &self,
        owner: Owner<'_>,
        files: &impl Continuations,
    ) -> rusqlite::Result<Vec<Question>> {
        let (cond, args) = self.owner_clause(owner, files)?;
        let sql = format!(
            "SELECT {COLUMNS} FROM {VIEW} WHERE {cond} AND status = 'answered' AND acknowledged_at IS NULL ORDER BY id"
        );
        self.questions(&sql, args)
    }

    /// Open questions plus open alerts per pane (the sidebar count).
    pub fn open_counts(&self) -> rusqlite::Result<BTreeMap<String, i64>> {
        let mut stmt = self.db().prepare(
            "SELECT pane_id, COUNT(*) FROM (\
             SELECT pane_id FROM questions WHERE status = 'open' AND pane_id IS NOT NULL \
             UNION ALL SELECT pane_id FROM alerts WHERE status = 'open' AND pane_id IS NOT NULL\
             ) GROUP BY pane_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Every pane a question was asked from.
    pub fn known_panes(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .db()
            .prepare("SELECT DISTINCT pane_id FROM questions WHERE pane_id IS NOT NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }
}
