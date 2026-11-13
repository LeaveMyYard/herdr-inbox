//! The `ask_operator` call's `tool_use_id` (schema revision 3,
//! `question_ext.tool_use_id`): the id of the `tool_use` block that asked, in
//! the asking agent's transcript. Claude Code sends it in the call's `_meta`
//! (`claudecode/toolUseId`); another host, or the Python server, files none.
//! A chat view anchors the question on that block with it.

use rusqlite::{OptionalExtension, params};

use super::Store;

/// The `_meta` key Claude Code puts the id under.
pub const META_KEY: &str = "claudecode/toolUseId";

/// The longest id kept. Claude Code's are `toolu_` and 24 or so characters.
pub const MAX_LEN: usize = 128;

/// `id` if it looks like a tool-use id: 1 to [`MAX_LEN`] ASCII letters,
/// digits, `_` and `-`. Anything else is dropped, not stored: the feed
/// publishes this id, so it must never carry text the agent controls.
pub fn checked(id: Option<&str>) -> Option<&str> {
    id.filter(|s| {
        (1..=MAX_LEN).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}

impl Store {
    /// Records `id` as question `qid`'s call. Its `question_ext` row must exist.
    pub fn set_tool_use_id(&self, qid: i64, id: &str) -> rusqlite::Result<()> {
        self.db().execute(
            "UPDATE question_ext SET tool_use_id = ? WHERE id = ?",
            params![id, qid],
        )?;
        Ok(())
    }

    /// Question `qid`'s call id, if one was recorded.
    pub fn tool_use_id(&self, qid: i64) -> rusqlite::Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT tool_use_id FROM question_ext WHERE id = ?",
                params![qid],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }
}

#[cfg(test)]
mod tests;
