//! The schema revision stamp (Python `store.MIGRATION_REVISION`,
//! `stamped_revision`; plan §2.3, "The encoding").
//!
//! `PRAGMA user_version` holds `REVISION_BASE + revision`, a negative 32-bit
//! value; anything else (a legacy crc32 stamp, 0) is "unknown". Only the
//! migrating open (`migrate`, the worker and the popup) writes the stamp;
//! the hooks never do.

/// The schema revision of the Python bridge (0.10.6): what `tests/fixtures/schema.json`
/// and `migrate.json` capture. Python stays there.
pub const BRIDGE_REVISION: i64 = 1;

/// The schema revision this code uses: the bridge's plus [`super::owned`].
/// 2: `question_host`, `alert_host`, `question_ext.retry_count` (B2).
/// 3: `question_ext.tool_use_id`, the `ask_operator` call's id in its transcript.
pub const MIGRATION_REVISION: i64 = 3;

/// The sign bit marks a revision: `-(1 << 31)`.
pub const REVISION_BASE: i64 = -(1 << 31);

/// The `user_version` that says "migrated to [`MIGRATION_REVISION`]" (Python
/// `revision_stamp()`). Only a migrating open writes it.
pub const STAMP: i64 = REVISION_BASE + MIGRATION_REVISION;

/// The revision a `user_version` names, or `None` for a legacy crc32 stamp
/// (always 0..2^31-1) or 0. `user_version` is a signed 32-bit value.
pub fn stamped_revision(user_version: i64) -> Option<i64> {
    (user_version < 0).then(|| user_version - REVISION_BASE)
}

#[cfg(test)]
mod tests;
