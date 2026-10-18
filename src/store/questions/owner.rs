//! Whose questions a query is about (Python `Store._owner_clause`): a
//! session's (and the sessions it continued), else, for an agent without a
//! session, its pane's.

use rusqlite::types::Value;

use crate::store::Store;
use crate::store::family::{Continuations, family};

/// The asker an owner query is about.
#[derive(Clone, Copy, Debug)]
pub struct Owner<'a> {
    /// Its session, if it has one.
    pub session_id: Option<&'a str>,
    /// Its herdr pane.
    pub pane_id: Option<&'a str>,
}

impl Store {
    /// The SQL condition and its arguments for `owner`'s questions.
    pub(crate) fn owner_clause(
        &self,
        owner: Owner<'_>,
        files: &impl Continuations,
    ) -> rusqlite::Result<(String, Vec<Value>)> {
        if let Some(session) = owner.session_id.filter(|s| !s.is_empty()) {
            let fam = family(self.db(), session, files)?;
            let marks = vec!["?"; fam.len()].join(",");
            return Ok((
                format!("session_id IN ({marks})"),
                fam.into_iter().map(Value::Text).collect(),
            ));
        }
        if let Some(pane) = owner.pane_id.filter(|p| !p.is_empty()) {
            return Ok((
                "session_id IS NULL AND pane_id = ?".into(),
                vec![Value::Text(pane.into())],
            ));
        }
        Ok(("0".into(), Vec::new()))
    }
}
