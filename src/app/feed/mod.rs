//! The feed's rows and the snapshot they make (`app-feed-v1.md`). Every row is
//! built here, for the writer and for a verb alike, so a verb's token is the
//! one the writer published.
//!
//! - [`token`]: a row's revision token.
//! - [`text`]: the size limits on long text.
//! - [`asker`], [`group`], [`failed`], [`alert`], [`remote`]: one row each.
//! - [`answered`]: what was answered lately, beside the entries.
//! - [`fit`]: what stays under 1 MiB.
//! - [`snapshot`]: the whole feed, from one read transaction.

pub mod alert;
pub mod answered;
pub mod asker;
pub mod failed;
pub mod fit;
pub mod group;
pub mod remote;
pub mod snapshot;
pub mod text;
pub mod token;
