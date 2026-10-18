//! What the `herdr-inbox` binary's subcommands share (plan §2.2): the inbox
//! database as the hooks use it ([`store`]), the Claude Code lookups they need
//! ([`claude`]), and small helpers. The binary is `src/main.rs`; this library
//! is not published.
//!
//! During the migration Python and Rust processes share one database (plan
//! §5.4), so every rule here is the Python bridge's (0.10.6+) rule, ported
//! as it is; each item names the Python it ports.

pub mod claude;
pub mod clock;
pub mod delivery;
pub mod hook;
pub mod starts;
pub mod store;
pub mod text;
pub mod token;
