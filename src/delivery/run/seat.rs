//! Where a pass reaches its agent: a herdr pane, or a terminal of herdr-core
//! ([`crate::delivery::terminal`]). The decision is the same for both; only
//! how the agent, the input box and the panel are read and how the text goes
//! differ.
//!
//! **Core typing.** The pass looks once more right before the send: the same
//! process, the same conversation. `hc send --if idle` then settles the status
//! itself, in the core, so what it refuses was never typed and costs no attempt.

use herdr_agent_kit::herdr_api::Sanitized;
use inbox_core::deliver::{
    Agent, BLOCKED_WAIT, Composer, NO_AGENT, Prompted, not_idle, terminal_unread,
};

use super::Ctx;
use super::look::PaneLook;
use crate::delivery::pane::{composer, live_agent, panel, prompt};
use crate::delivery::screen::{composer_state, panel_focused};
use crate::delivery::terminal::{REPLACED, Seated, Seen};
use crate::host::core::hc::Sent;
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
        match self.ctx.core.see(&self.pane) {
            Seen::Here(seated) => {
                let agent = seated.agent.clone();
                self.seated = Some(*seated);
                Some(agent)
            }
            Seen::Gone => None,
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
        match self.ctx.core.see(&self.pane) {
            Seen::Here(now) if same(&now, &before) => {}
            Seen::Here(_) => return Prompted::Refused(terminal_unread(REPLACED)),
            Seen::Gone => return Prompted::Failed(format!("{NO_AGENT} {}", self.pane)),
            Seen::Unread(why) => return Prompted::Refused(terminal_unread(&why)),
        }
        let text = Sanitized::new(text);
        match self.ctx.core.send(&self.pane, text.as_str()) {
            Sent::Typed => Prompted::Sent,
            Sent::Blocked => Prompted::Refused(BLOCKED_WAIT.into()),
            Sent::NotIdle => Prompted::Refused(not_idle(&before.agent.status)),
            Sent::Gone => Prompted::Failed(format!("{NO_AGENT} {}", self.pane)),
            Sent::Unread(why) => Prompted::Refused(terminal_unread(why)),
            Sent::Failed(error) => Prompted::Failed(error),
        }
    }
}

fn same(a: &Seated, b: &Seated) -> bool {
    (a.pid, &a.start, &a.agent.session) == (b.pid, &b.start, &b.agent.session)
}
