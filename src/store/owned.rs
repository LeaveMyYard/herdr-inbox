//! What Rust added to the schema after the Python bridge's revision (plan
//! herdr-app-inbox §3.2): `tests/fixtures/migrate-rust.json`.
//!
//! Python stays at [`super::revision::BRIDGE_REVISION`] and its captured
//! fixtures stay as they are, so this comes on top of them. Additive only, in
//! side tables and in `question_ext`: a Python process finds a stamp above its
//! own, leaves the schema alone and reads none of it.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::revision::MIGRATION_REVISION;

const OWNED_JSON: &str = include_str!("../../tests/fixtures/migrate-rust.json");

/// The statements and columns of [`MIGRATION_REVISION`] past the bridge.
#[derive(Debug, Deserialize)]
pub struct Owned {
    /// The revision they bring a database to.
    pub revision: i64,
    /// Every `CREATE ... IF NOT EXISTS`, in order.
    pub statements: Vec<String>,
    /// Per table, the columns added to one the bridge already has, with their declarations.
    pub added_columns: Vec<(String, Vec<(String, String)>)>,
    /// The new tables' columns.
    pub tables: BTreeMap<String, Vec<String>>,
}

impl Owned {
    /// Is `table.column` one of these additions?
    pub fn has(&self, table: &str, column: &str) -> bool {
        self.tables.contains_key(table)
            || self
                .added_columns
                .iter()
                .any(|(t, added)| t == table && added.iter().any(|(name, _)| name == column))
    }
}

/// This revision's additions (parsed once).
pub fn owned() -> &'static Owned {
    static OWNED: OnceLock<Owned> = OnceLock::new();
    OWNED.get_or_init(|| {
        let doc: Owned = serde_json::from_str(OWNED_JSON)
            .expect("tests/fixtures/migrate-rust.json is this revision's additions");
        assert_eq!(
            doc.revision, MIGRATION_REVISION,
            "migrate-rust.json is another revision's"
        );
        doc
    })
}
