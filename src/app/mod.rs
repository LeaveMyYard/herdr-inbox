//! The inbox as herdr-app's mods see it (`docs/protocol/app-feed-v1.md`,
//! `app-commands-v1.md`): Rust owns every decision and every write, and a mod
//! reads one file and acts through commands.
//!
//! - [`feed`]: the rows, their tokens and the snapshot, read from the database.
//! - [`writer`]: `app-feed`, the process that publishes it.
//! - [`verbs`]: `app <verb>`, how a mod acts.

pub mod feed;
pub mod verbs;
pub mod writer;
