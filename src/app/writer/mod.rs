//! `herdr-inbox app-feed`: one detached writer per app home, and the
//! `--ensure` that starts it (`app-feed-v1.md`, "Runtime"). The writer only
//! reads the inbox: it calls neither herdr nor `hc`, starts no worker and
//! writes nothing but the files in [`files::Paths`]. Nothing is ever killed:
//! a writer leaves when `control.json` or `stop.request` says so.
//!
//! - [`cli`]: the command line.
//! - [`files`]: the paths, the locks and an atomic write.
//! - [`control`], [`ledger`], [`record`]: `control.json`, `start.json`, `writer.json`.
//! - [`ensure`]: the proven, bounded start.
//! - [`run`]: the writer's life, [`source`] what it reads and [`idle`] when it leaves.

pub mod cli;
pub mod control;
pub mod ensure;
pub mod files;
pub mod idle;
pub mod ledger;
pub mod record;
pub mod run;
pub mod source;
