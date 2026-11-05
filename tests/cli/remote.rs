//! The remote side's part of the CLI golden (tools/capture_cli_remote.py):
//! its database, its masking, its tables, and the puller's view of every
//! export: what Rust's `inbox export` printed and what Python's printed
//! must both pass the puller's validation (`remote::validate`) and mirror
//! the same rows.

use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_inbox::remote::validate::validate_export;
use herdr_inbox::store::Store;
use regex::{Captures, Regex};
use serde_json::Value;

use crate::common::migrated;
use crate::store_golden::tables as dump;

/// The remote side's database id once the golden fixes it (capture_cli_remote `DB`).
const DB: &str = "d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0";
/// When its questions and alerts were created (`CREATED`): never masked.
const CREATED: &str = "\"2001-01-01T00:00:00+00:00\"";
const TABLES: [&str; 6] = [
    "questions",
    "question_ext",
    "question_groups",
    "alerts",
    "applied_requests",
    "changes",
];

static TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+00:00""#).unwrap());
static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\{"version": "[^"]*""#).unwrap());
static DB_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""db_id": "([0-9a-f]{32})""#).unwrap());

/// A new database, migrated (the `fresh` op).
pub fn fresh(dir: &Path) {
    for name in [
        "inbox.db",
        "inbox.db-wal",
        "inbox.db-shm",
        "inbox.db-journal",
    ] {
        let _ = std::fs::remove_file(dir.join(name));
    }
    migrated(dir);
}

/// An export's or apply's stdout as the golden keeps it (capture_cli_remote `mask`).
pub fn mask(stdout: &str) -> String {
    let out = TIME.replace_all(stdout, |c: &Captures<'_>| {
        if &c[0] == CREATED {
            c[0].to_string()
        } else {
            "\"<time>\"".to_string()
        }
    });
    let out = VERSION.replace(&out, r#"{"version": "<version>""#);
    let out = DB_ID.replace_all(&out, |c: &Captures<'_>| {
        if &c[1] == DB {
            c[0].to_string()
        } else {
            r#""db_id": "<db_id>""#.to_string()
        }
    });
    // a JSON parser's reason: Python's json words it, serde_json differently
    out.lines()
        .map(|line| {
            let head = "\"bad request: ";
            match line.find(head) {
                Some(at) if !line[at..].starts_with("\"bad request: not an object\"") => {
                    format!("{}{head}<reason>\"}}\n", &line[..at])
                }
                _ => format!("{line}\n"),
            }
        })
        .collect()
}

/// The rows after a step of the remote side, and the database's id.
pub fn tables(store: &Store) -> Value {
    let mut out = dump(store, &TABLES);
    let id: Option<String> = store
        .db()
        .query_row("SELECT value FROM meta WHERE key = 'db_id'", [], |r| {
            r.get(0)
        })
        .ok();
    let id = id.map(|i| if i == DB { i } else { "<db_id>".into() });
    out.as_object_mut()
        .unwrap()
        .insert("db_id".into(), id.into());
    out
}

/// What a puller's mirror holds after one pull of `export` into a fresh
/// mirror at a fixed time: its questions and alerts (times masked as in
/// the golden), the machine's cursor, and the notices it sends.
fn mirrored(export: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
    let data = validate_export(export).unwrap();
    let rec = store.record_machine("m1", Some("qnap"), None).unwrap();
    let at: SystemTime = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let notices = store.apply_pull(&rec, &data, at).unwrap();
    let rows = |sql: &str| -> Vec<String> {
        let mut stmt = store.db().prepare(sql).unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| mask(&r.unwrap()).trim_end().to_string())
            .collect()
    };
    serde_json::json!({
        "questions": rows("SELECT machine_id || ' ' || db_id || ' ' || remote_id || ' ' || qkey || ' ' || status \
                           || ' ' || data_json FROM remote_questions ORDER BY remote_id"),
        "alerts": rows("SELECT machine_id || ' ' || alert_id || ' ' || akey || ' ' || status || ' ' || data_json \
                        FROM remote_alerts ORDER BY alert_id"),
        "machine": rows("SELECT db_id || ' ' || cursor_seq FROM remote_machines"),
        "notices": notices.iter().map(|n| format!("{} | {}", n.title, n.body)).collect::<Vec<_>>(),
    })
}

/// Rust's export (as printed, `ours`) and Python's (as the golden keeps it,
/// `theirs`) both validate, and mirror the same rows. Python's first export
/// of an empty database has its random id masked: only ours is validated.
pub fn cross_check(ours: &str, theirs: &str, what: &str) {
    let data =
        validate_export(ours).unwrap_or_else(|e| panic!("{what}: Rust's export refused: {e:?}"));
    // the version the golden masks: the plugin's (Python's `__version__`, the same by release)
    assert_eq!(
        data.version.as_deref(),
        Some(env!("CARGO_PKG_VERSION")),
        "{what}"
    );
    if theirs.contains("<db_id>") {
        return;
    }
    validate_export(theirs).unwrap_or_else(|e| panic!("{what}: Python's export refused: {e:?}"));
    assert_eq!(
        mirrored(ours),
        mirrored(theirs),
        "{what}: the mirrors differ"
    );
}
