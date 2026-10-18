//! `herdr-inbox hook rewake-watch` (Python `rewake.run`): keystroke-free
//! delivery to an IDLE Claude session.
//!
//! Claude Code runs a hook marked `asyncRewake: true` in the background;
//! when it exits 2, its output wakes the session as a system reminder and
//! starts a turn. Every `Stop` arms one watcher for its session. It polls the
//! database read-only, asks herdr whether the pane's agent is at rest with
//! this very session, and then delivers through the hook's own path
//! (`hook::deliver`, claims and all) and exits [`WAKE_EXIT`]; the hooks.json
//! wrapper maps exactly that code to 2. The decisions are
//! `inbox_core::rewake`'s; this module does the IO.
//!
//! One watcher per session: each new one writes its token to
//! `<session>.owner`, and an older one that sees another token exits. While
//! alive it holds `<session>.lock` (flock): the delivery worker does not type
//! into an idle session whose watcher is alive.

mod clock;
mod owner;
mod pane;
mod watch;

pub use clock::{Clock, SystemClock};
pub use watch::{Watcher, run};

/// The watcher's private "wake" exit code; the hooks.json wrapper maps it,
/// and only it, to Claude Code's exit 2.
pub const WAKE_EXIT: i32 = 3;

/// Seconds between database looks (read-only); the wake follows within one.
pub const WATCH_POLL: f64 = 2.0;
