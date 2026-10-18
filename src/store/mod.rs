//! The inbox database as the hooks and the rewake watcher use it (plan §2.3,
//! "The inbox binary's store module"; §5.4).
//!
//! The rules of this side of the database, shared with the Python bridge:
//!
//! - **No migration, no stamp.** Only the worker and the popup migrate. A
//!   hook opens the database, checks that the schema it uses is there
//!   ([`readiness`]) and does nothing if not. Nothing here writes
//!   `user_version`.
//! - **Bounded lookups.** A busy or locked database never holds a session:
//!   the hook's lookups give up after a wall-clock deadline ([`busy`]).
//! - **Never create the database.** Every open needs the file to exist.
//!
//! The SQL is Python's (`store.py`, `hook.py`), statement for statement.

pub mod busy;
pub mod candidates;
pub mod claims;
pub mod connect;
pub mod family;
pub mod liveness;
pub mod pending;
pub mod question;
pub mod readiness;
pub mod revision;
pub mod sessions;

use std::path::PathBuf;

use herdr_agent_kit::internal::plugin_paths::inbox;
use herdr_agent_kit::plugin_env::PluginEnv;

pub use claims::Claim;
pub use connect::{OpenError, Store};
pub use question::Question;

/// After this many hook sends that never showed up in the conversation, an
/// answer is left to typing (Python `store.MAX_HOOK_MISSES`).
pub const MAX_HOOK_MISSES: i64 = 2;

/// The database: `$HERDR_INBOX_DB`, else
/// `${XDG_STATE_HOME:-~/.local/state}/herdr-inbox/inbox.db` (Python
/// `store.db_path`).
pub fn db_path(env: &PluginEnv) -> PathBuf {
    inbox::db_path(env)
}
