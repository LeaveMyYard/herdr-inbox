//! The launcher contract (plan §4 item 4): one number for everything the
//! Claude Code side's `scripts/launch` relies on.
//!
//! It covers the hook subcommand's payload handling, its output, its exit
//! codes (0, and the rewake watcher's 3), the MCP tool schemas, and the
//! subcommands the launcher starts. `scripts/launch` runs only a binary that
//! prints the number it carries, so a Claude plugin and a binary of different
//! versions work together as long as the number matches. Raise it, here and
//! in `scripts/launch` together, only for a change the other side cannot take.
//!
//! 2 (P5-I): `mcp`, `worker` and the CLI are there. A binary of contract 1
//! has the hooks only and would print its version for `mcp`, which is a dead
//! MCP server; a launcher of contract 1 runs only the hooks.

/// This binary's contract (`herdr-inbox contract`).
pub const CONTRACT: u32 = 2;
