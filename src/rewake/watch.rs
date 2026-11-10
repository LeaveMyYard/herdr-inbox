//! The watcher's loop: the IO around `inbox_core::rewake`'s decisions.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use herdr_agent_kit::agent_procs::Pid;
use herdr_agent_kit::agent_procs::claude::owning_claude_here;
use herdr_agent_kit::agent_procs::pid_alive;
use herdr_agent_kit::agent_session::SessionId;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::internal::plugin_paths::inbox;
use herdr_agent_kit::plugin_env::{LockFile, PluginEnv};
use inbox_core::rewake::{self, LOCK_WAIT, PaneAgent, Verdict, Watch};

use super::WATCH_POLL;
use super::clock::Clock;
use super::owner::Owner;
use super::pane::{pane_agent, terminal_agent};
use crate::claude::Transcripts;
use crate::delivery::terminal::{Terminals, live as live_terminals};
use crate::hook::deliver::deliver;
use crate::hook::payload::Payload;
use crate::store::candidates::has_candidates;
use crate::store::db_path;
use crate::store::liveness::{Stamp, touch};

/// How often the lock is tried while the superseded watcher lets go.
const LOCK_POLL: Duration = Duration::from_millis(100);

/// Everything one watcher works with.
pub struct Watcher<'a> {
    /// The inbox database.
    pub db: PathBuf,
    /// The inbox's state directory (`rewake/`, `starts/`).
    pub state_dir: PathBuf,
    /// The pane, from `HERDR_PANE_ID`.
    pub pane: Option<String>,
    /// The core terminal, proved: it goes before the pane.
    pub terminal: Option<String>,
    /// The herdr this process belongs to.
    pub herdr: Herdr,
    /// herdr-core's terminals.
    pub core: Box<dyn Terminals>,
    /// Claude Code's transcripts.
    pub files: Transcripts,
    /// The Claude process the watch belongs to (`None`: unknown, never "gone").
    pub owner: Option<Pid>,
    /// Time.
    pub clock: &'a dyn Clock,
}

impl<'a> Watcher<'a> {
    /// A watcher in this process's environment, for the proved `terminal` if any.
    pub fn from_env(
        env: &PluginEnv,
        terminal: Option<String>,
        clock: &'a dyn Clock,
    ) -> Watcher<'a> {
        let herdr = crate::trace::observed(Herdr::from_env());
        Watcher {
            db: db_path(env),
            state_dir: inbox::state_dir(env),
            pane: env.pane_id().map(str::to_owned),
            terminal,
            core: Box::new(live_terminals(env, herdr.clone())),
            herdr,
            files: Transcripts::from_env(),
            owner: owning_claude_here(),
            clock,
        }
    }
}

/// Watches for this session's answers: the wake text, or `None` (the watch
/// ended without one).
pub fn run(w: &Watcher<'_>, payload: &Payload) -> Option<String> {
    let session = payload.session_id.as_deref().and_then(SessionId::parse);
    let seat = w.terminal.as_deref().or(w.pane.as_deref());
    if !rewake::start(session.is_some(), seat.is_some(), payload.agent_id) {
        return None;
    }
    let sid = session?;
    let dir = w.state_dir.join("rewake");
    let owner = Owner::claim(&dir.join(format!("{sid}.owner"))).ok()?;
    let started = w.clock.now();
    let keep = || rewake::keep_waiting_for_lock(started, w.clock.now(), owner.still_owner());
    let wait = Duration::from_secs_f64(LOCK_WAIT);
    let _lock = LockFile::new(dir.join(format!("{sid}.lock")))
        .exclusive_within(wait, LOCK_POLL, keep)
        .ok()??;
    // the session id as the payload gave it: the hook path and the database use it as is
    let session = payload.session_id.as_deref()?;
    let mut watch = Watch::new(w.clock.now());
    loop {
        if watch.begin_poll(w.clock.now(), owner.still_owner(), pid_alive(w.owner)) == Verdict::End
        {
            return None;
        }
        let pending = has_candidates(&w.db, session, &w.files);
        if watch.look(pending, w.clock.now()) {
            let agent = match (w.terminal.as_deref(), w.pane.as_deref()) {
                (Some(terminal), _) => terminal_agent(&*w.core, terminal),
                (None, Some(pane)) => pane_agent(&w.herdr, &w.files, &w.state_dir, pane),
                (None, None) => None,
            };
            let view = agent.as_ref().map(|(s, status)| PaneAgent {
                session_id: s.as_deref(),
                status: *status,
            });
            let ready = rewake::ready(view, sid.as_str());
            match watch.after_look(pending, ready, w.clock.now()) {
                Verdict::End => return None, // also with nothing pending: a /clear or /resume left this watch behind
                Verdict::Deliver => {
                    let text = deliver(&w.db, &w.files, payload, false);
                    if Watch::after_delivery(text.is_some()) == Verdict::Wake {
                        return text;
                    }
                }
                Verdict::KeepWatching | Verdict::Wake => {}
            }
        }
        touch(&w.db, session, Stamp::WatchSeen, SystemTime::now()); // the IDLE gate's own stamp
        w.clock.sleep(WATCH_POLL);
    }
}
