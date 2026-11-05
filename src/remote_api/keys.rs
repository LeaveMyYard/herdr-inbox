//! The identities `inbox export` hands out and `inbox apply` checks (Python
//! `remote_api.db_id`, `qkey`, `alerts.akey`): the database's random id, and
//! a key per question and per alert that a recreated database cannot repeat.

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::store::Question;
use crate::store::alerts::Alert;

/// This database's random identity (Python `db_id`): created with the first
/// call (a recreated database gets a new one), 32 lowercase hex digits.
///
/// Python writes `uuid.uuid4().hex`; this writes 16 bytes of SQLite's
/// `randomblob` (seeded from the OS), as hex: the same shape to every
/// reader, which only compares it and checks that it is 8 to 64 hex digits.
pub fn db_id(db: &Connection) -> rusqlite::Result<String> {
    if let Some(id) = read(db)? {
        return Ok(id);
    }
    db.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('db_id', lower(hex(randomblob(16))))",
        [],
    )?;
    read(db)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

fn read(db: &Connection) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT value FROM meta WHERE key = 'db_id'", [], |r| {
        r.get(0)
    })
    .optional()
}

/// A question's identity beyond its number (Python `qkey`).
pub fn qkey(db_id: &str, q: &Question) -> String {
    let session = q.session_id.as_deref().unwrap_or("");
    hex32(&format!("{db_id}|{}|{}|{session}", q.created_at, q.norm))
}

/// An alert's identity beyond its number (Python `alerts.akey`).
pub fn akey(db_id: &str, a: &Alert) -> String {
    hex32(&format!(
        "{db_id}|alert|{}|{}|{}",
        a.id, a.created_at, a.title
    ))
}

/// `hashlib.sha256(text.encode()).hexdigest()[:32]`.
fn hex32(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests;
