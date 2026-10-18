//! The launcher contract (plan §4 item 4): one number for everything the
//! Claude Code side's `scripts/launch` relies on.
//!
//! It covers the hook subcommand's payload handling, its output, its exit
//! codes (0, and the rewake watcher's 3) and, once the MCP server is Rust,
//! the MCP tool schemas. `scripts/launch` runs only a binary that prints the
//! number it carries, so a Claude plugin and a binary of different versions
//! work together as long as the number matches. Raise it, here and in
//! `scripts/launch` together, only for a change the other side cannot take.

/// This binary's contract (`herdr-inbox contract`).
pub const CONTRACT: u32 = 1;
