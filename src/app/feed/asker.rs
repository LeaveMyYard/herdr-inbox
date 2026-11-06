//! Who asked (`app-feed-v1.md`, "Asker"). A core terminal has a side-table
//! row, a herdr pane has none, and `unknown` is a row with neither.

use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::store::Store;
use crate::store::alerts::Alert;
use crate::store::hosts::CORE;
use crate::store::question::Question;
use crate::tabs::{DEFAULT_PREFIX, base_label};

/// `asker` of `q`.
pub fn of_question(store: &Store, q: &Question) -> rusqlite::Result<Value> {
    let terminal = terminal(store, "question_host", q.id)?;
    Ok(asker(
        terminal,
        q.pane_id.as_deref(),
        q.tab_label.as_deref(),
        q.cwd.as_deref(),
    ))
}

/// `asker` of alert `a`. `cwd` is always null: alerts store none.
pub fn of_alert(store: &Store, a: &Alert) -> rusqlite::Result<Value> {
    let terminal = terminal(store, "alert_host", a.id)?;
    Ok(asker(
        terminal,
        a.pane_id.as_deref(),
        a.tab_label.as_deref(),
        None,
    ))
}

/// The core terminal row `id` was raised from, in side table `table`.
fn terminal(store: &Store, table: &str, id: i64) -> rusqlite::Result<Option<String>> {
    let sql = format!("SELECT terminal FROM {table} WHERE id = ? AND host = ?");
    let found = store
        .db()
        .query_row(&sql, rusqlite::params![id, CORE], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()?;
    Ok(found.flatten().filter(|t| !t.is_empty()))
}

fn asker(
    terminal: Option<String>,
    pane: Option<&str>,
    tab_label: Option<&str>,
    cwd: Option<&str>,
) -> Value {
    let label = tab_label.map(|l| base_label(l, &[DEFAULT_PREFIX]));
    match (terminal, pane.filter(|p| !p.is_empty())) {
        (Some(t), _) => {
            json!({"host": CORE, "pane": null, "terminal": t, "tab_label": null, "cwd": cwd})
        }
        (None, Some(p)) => {
            json!({"host": "herdr-tui", "pane": p, "terminal": null, "tab_label": label, "cwd": cwd})
        }
        (None, None) => {
            json!({"host": "unknown", "pane": null, "terminal": null, "tab_label": label, "cwd": cwd})
        }
    }
}

#[cfg(test)]
mod tests;
