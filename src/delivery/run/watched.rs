//! Whether an idle session's rewake watcher is alive: the worker leaves that
//! session's answers to it.

use herdr_agent_kit::agent_session::SessionId;
use herdr_agent_kit::plugin_env::LockFile;

use super::HOOK_LIVE;
use super::look::PaneLook;

impl PaneLook<'_, '_> {
    /// Python `_watched`: the idle session's asyncRewake watcher stamped
    /// itself recently and holds its lock.
    pub(super) fn watched_now(&mut self) -> bool {
        let session = self.session().map(String::from);
        let live = self.ctx.store.watch_live(session.as_deref(), HOOK_LIVE);
        if !self.ok(live) {
            return false;
        }
        let Some(sid) = session.as_deref().and_then(SessionId::parse) else {
            return false;
        };
        LockFile::new(
            self.ctx
                .state_dir
                .join("rewake")
                .join(format!("{sid}.lock")),
        )
        .held_elsewhere_if_exists()
    }
}
