//! Where a pass reaches its agent: a herdr pane, or a terminal of herdr-core
//! ([`crate::delivery::terminal`]). The decision is the same for both; only
//! how the agent, the input box and the panel are read and how the text goes
//! differ.
//!
//! **Core typing.** The pass looks once more right before the send: the same
//! process, the same conversation. `hc send --if idle` then settles the status
//! itself, in the core, so what it refuses was never typed and costs no attempt.
//! At `done`, which `--if` has no word for, the send is plain, on that last look.

use std::path::{Path, PathBuf};

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::Sanitized;
use inbox_core::deliver::{
    Agent, BLOCKED_WAIT, Composer, NO_AGENT, Prompted, not_idle, terminal_unread,
};

use super::Ctx;
use super::look::PaneLook;
use crate::claude::Transcripts;
use crate::delivery::pane::{composer, live_agent, panel, prompt};
use crate::delivery::plans::question_family;
use crate::delivery::screen::{composer_state, panel_focused};
use crate::delivery::terminal::{REPLACED, SHARED, Seated, Seen};
use crate::host::core::hc::Sent;
use crate::store::sessions::{record_session, session_transcript};
use crate::store::{Question, Store};

/// Where the agent of an answer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seat {
    /// A herdr pane; `None`: no pane known.
    Pane(Option<String>),
    /// A terminal of this machine's core.
    Terminal(String),
    /// A row of the core with no terminal to go to here: nothing to do.
    Nowhere,
}

/// Where `q`'s agent is, by what was stored when it asked.
pub fn seat_of(store: &Store, q: &Question) -> rusqlite::Result<Seat> {
    if q.pane_id.is_some() {
        return Ok(Seat::Pane(q.pane_id.clone()));
    }
    Ok(match store.core_seat(q.id)? {
        Some(seat) if seat.machine.is_none() => Seat::Terminal(seat.terminal),
        Some(_) => Seat::Nowhere, // another machine's: out of scope (plan §3.5)
        None => Seat::Pane(None),
    })
}

/// A pass's answers per seat, in the order the seats first appear; a row with
/// nowhere to go is left out, untouched.
pub fn by_seat<'q>(
    store: &Store,
    qs: &'q [Question],
) -> rusqlite::Result<Vec<(Seat, Vec<&'q Question>)>> {
    let mut seats: Vec<(Seat, Vec<&'q Question>)> = Vec::new();
    for q in qs {
        let seat = seat_of(store, q)?;
        if seat == Seat::Nowhere {
            continue;
        }
        match seats.iter_mut().find(|(s, _)| *s == seat) {
            Some((_, list)) => list.push(q),
            None => seats.push((seat, vec![q])),
        }
    }
    Ok(seats)
}

fn screen_box(ctx: &Ctx<'_>, terminal: &str) -> Composer {
    ctx.core
        .screen(terminal)
        .map_or(Composer::Unknown, |screen| composer_state(&screen))
}

impl PaneLook<'_, '_> {
    fn on_terminal(&self) -> bool {
        self.core
    }

    /// The agent now, and what stands in for one that could not be looked at.
    pub(super) fn read_agent(&mut self) -> Option<Agent> {
        if !self.on_terminal() {
            let (ctx, pane) = (&self.ctx, self.pane.as_str());
            return live_agent(ctx.herdr, ctx.files, ctx.state_dir, pane);
        }
        let items: Vec<&Question> = self.qs.values().copied().collect();
        let configs = configs_of(self.ctx.store, &items);
        let configs = self.ok(configs);
        match self.ctx.core.see(&self.pane, &configs) {
            Seen::Here(seated) => {
                let remembered = remember(self.ctx, &seated);
                self.ok(remembered);
                let agent = seated.agent.clone();
                self.seated = Some(*seated);
                Some(agent)
            }
            Seen::Gone => None,
            Seen::Shared(_) => {
                self.unread = Some(SHARED.into());
                None
            }
            Seen::Unread(why) => {
                self.unread = Some(why);
                None
            }
        }
    }

    /// Why the last [`PaneLook::read_agent`] found nobody, when it could not look.
    pub(super) fn read_failed(&mut self) -> Option<String> {
        self.unread.take().map(|why| terminal_unread(&why))
    }

    pub(super) fn read_box(&self) -> Composer {
        if self.on_terminal() {
            screen_box(&*self.ctx, &self.pane)
        } else {
            composer(self.ctx.herdr, &self.pane)
        }
    }

    /// The conversation of `id` runs in another terminal: the row goes with it.
    pub(super) fn moved_terminal(&mut self, id: i64) -> bool {
        let q = self.q(id);
        let found = terminal_of(&*self.ctx, q, Some(&self.pane));
        let Some(terminal) = self.ok(found) else {
            return false;
        };
        let moved = self
            .ctx
            .store
            .move_to_terminal(id, &terminal, self.claims.get(&id));
        self.ok(moved)
    }

    pub(super) fn read_panel(&self) -> bool {
        if self.on_terminal() {
            let screen = self.ctx.core.screen(&self.pane);
            screen.is_some_and(|s| panel_focused(&s))
        } else {
            panel(self.ctx.herdr, &self.pane)
        }
    }

    /// Types `text`.
    pub(super) fn send(&mut self, text: &str) -> Prompted {
        if !self.on_terminal() {
            return prompt(self.ctx.herdr, &self.pane, text)
                .map_or(Prompted::Sent, Prompted::Failed);
        }
        let Some(before) = self.seated.clone() else {
            return Prompted::Refused(terminal_unread(REPLACED));
        };
        // the same process, the same conversation: never a hand-me-down of the look above
        let items: Vec<&Question> = self.qs.values().copied().collect();
        let configs = configs_of(self.ctx.store, &items);
        let configs = self.ok(configs);
        let status = match self.ctx.core.see(&self.pane, &configs) {
            Seen::Here(now) if same(&now, &before) => now.agent.status,
            Seen::Here(_) => return Prompted::Refused(terminal_unread(REPLACED)),
            Seen::Gone => return Prompted::Failed(format!("{NO_AGENT} {}", self.pane)),
            Seen::Shared(_) => return Prompted::Refused(terminal_unread(SHARED)),
            Seen::Unread(why) => return Prompted::Refused(terminal_unread(&why)),
        };
        // the box once more, as late as this process can: DESIGN says what stays open
        if let Some(reason) = before.agent.held_by(screen_box(&*self.ctx, &self.pane)) {
            return Prompted::Refused(reason.into());
        }
        let text = Sanitized::new(text);
        match self.ctx.core.send(&self.pane, text.as_str(), &status) {
            Sent::Typed => Prompted::Sent,
            Sent::Blocked => Prompted::Refused(BLOCKED_WAIT.into()),
            Sent::NotIdle => Prompted::Refused(not_idle(&status)),
            Sent::Gone => Prompted::Failed(format!("{NO_AGENT} {}", self.pane)),
            Sent::Unread(why) => Prompted::Refused(terminal_unread(why)),
            Sent::Failed(error) => Prompted::Failed(error),
        }
    }
}

/// The conversation of `q` runs in this one other terminal now, proved by a look.
pub fn terminal_of(
    ctx: &Ctx<'_>,
    q: &Question,
    not: Option<&str>,
) -> rusqlite::Result<Option<String>> {
    let family = question_family(ctx.store.db(), ctx.files, q)?;
    let configs = configs_of(ctx.store, &[q])?;
    let mut hits = Vec::new();
    for terminal in ctx.core.list() {
        if Some(terminal.as_str()) == not {
            continue;
        }
        if let Seen::Here(seated) = ctx.core.see(&terminal, &configs)
            && seated.agent.session.is_some_and(|s| family.contains(&s))
        {
            hits.push(terminal);
        }
    }
    Ok(if hits.len() == 1 { hits.pop() } else { None })
}

/// The config directories `items` were asked under, by their transcripts: a
/// process in another one than this process's writes its records there.
fn configs_of(store: &Store, items: &[&Question]) -> rusqlite::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for q in items {
        let Some(session) = q.session_id.as_deref() else {
            continue;
        };
        let known = session_transcript(store.db(), session)?;
        let dir = known.as_deref().and_then(config_of);
        if let Some(dir) = dir.filter(|d| !dirs.contains(d)) {
            dirs.push(dir);
        }
    }
    Ok(dirs)
}

/// `<config>/projects/<slug>/<session>.jsonl`'s config directory.
fn config_of(transcript: &str) -> Option<PathBuf> {
    let projects = Path::new(transcript).parent()?.parent()?;
    let config = projects.parent()?;
    (projects.file_name()? == "projects").then(|| config.to_path_buf())
}

/// This process finds transcripts under its own directory only. What a terminal
/// of another one runs is written down, so that it finds that one's too.
fn remember(ctx: &Ctx<'_>, seated: &Seated) -> rusqlite::Result<()> {
    let (Some(config), Some(session)) = (&seated.config, &seated.agent.session) else {
        return Ok(());
    };
    let there = Transcripts::new(Some(ProjectsRoot::new(config.join("projects"))));
    let found = there.find(session, None).map(|p| p.display().to_string());
    record_session(ctx.store.db(), session, found.as_deref())
}

fn same(a: &Seated, b: &Seated) -> bool {
    (a.pid, &a.start, &a.agent.session) == (b.pid, &b.start, &b.agent.session)
}

#[cfg(test)]
mod tests;
