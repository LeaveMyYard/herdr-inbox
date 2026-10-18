//! Is the database's schema usable without migrating it? (Python
//! `Store.__init__` with `migrate=False`, `store.schema_missing`.)
//!
//! A database stamped with this code's revision or a higher one is used as it
//! is. Any other (a legacy crc32 stamp, 0, a lower revision) is checked,
//! read-only, against the schema this revision's migration produces: the
//! `tests/fixtures/schema.json` golden, captured from the Python migration
//! (`tools/capture_goldens.py --only schema`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use rusqlite::Connection;
use serde::Deserialize;

use super::revision::{MIGRATION_REVISION, stamped_revision};

/// The captured schema of [`MIGRATION_REVISION`].
const SCHEMA_JSON: &str = include_str!("../../tests/fixtures/schema.json");

/// The schema a migration to one revision produces.
#[derive(Debug, Deserialize)]
pub struct Expected {
    /// The revision it belongs to.
    pub revision: i64,
    /// Every table's columns.
    pub tables: BTreeMap<String, BTreeSet<String>>,
    /// Every index, trigger and view, by name.
    pub others: BTreeSet<String>,
    /// The statements that create it, in order (for tests).
    pub sql: Vec<String>,
}

/// The schema this code uses (parsed once).
pub fn expected() -> &'static Expected {
    static EXPECTED: OnceLock<Expected> = OnceLock::new();
    EXPECTED.get_or_init(|| {
        serde_json::from_str(SCHEMA_JSON)
            .expect("tests/fixtures/schema.json is the captured schema")
    })
}

/// What of this code's schema the database lacks, `table.column` first, then
/// the other objects, each part sorted; empty when it is usable. Nothing is
/// checked when the stamp names this revision or a higher one.
pub fn missing(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let user_version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if stamped_revision(user_version).is_some_and(|r| r >= MIGRATION_REVISION) {
        return Ok(Vec::new());
    }
    missing_from(db, expected())
}

/// What of `want` the database lacks (Python `schema_missing`): two
/// read-only SELECTs.
pub fn missing_from(db: &Connection, want: &Expected) -> rusqlite::Result<Vec<String>> {
    let mut have: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut stmt = db.prepare(
        "SELECT m.name, p.name FROM sqlite_master m JOIN pragma_table_info(m.name) p WHERE m.type = 'table'",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (table, column) = row?;
        have.entry(table).or_default().insert(column);
    }
    let mut stmt =
        db.prepare("SELECT name FROM sqlite_master WHERE type IN ('index', 'trigger', 'view')")?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<String>>>()?;
    let none = BTreeSet::new();
    let mut out: Vec<String> = want
        .tables
        .iter()
        .flat_map(|(table, cols)| {
            let got = have.get(table).unwrap_or(&none);
            cols.difference(got).map(move |c| format!("{table}.{c}"))
        })
        .collect();
    out.extend(want.others.difference(&names).cloned());
    Ok(out)
}
