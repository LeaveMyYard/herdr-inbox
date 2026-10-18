//! The schema revision stamp (Python `store.MIGRATION_REVISION`,
//! `stamped_revision`; plan §2.3, "The encoding").
//!
//! `PRAGMA user_version` holds `REVISION_BASE + revision`, a negative 32-bit
//! value; anything else (a legacy crc32 stamp, 0) is "unknown". Only read
//! here: the hooks never write the stamp.

/// The schema revision this code uses: the Python bridge's (0.10.6).
pub const MIGRATION_REVISION: i64 = 1;

/// The sign bit marks a revision: `-(1 << 31)`.
pub const REVISION_BASE: i64 = -(1 << 31);

/// The revision a `user_version` names, or `None` for a legacy crc32 stamp
/// (always 0..2^31-1) or 0. `user_version` is a signed 32-bit value.
pub fn stamped_revision(user_version: i64) -> Option<i64> {
    (user_version < 0).then(|| user_version - REVISION_BASE)
}

#[cfg(test)]
mod tests;
