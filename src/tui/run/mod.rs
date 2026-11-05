//! The popup's loop (Python `InboxApp`'s event loop): terminal input, the
//! poll timer and the background threads' reports in, model events out, the
//! model's effects carried out (`apply`), a frame after each.
//!
//! The model's contract, kept here:
//!
//! - **A fresh read before every key** ([`Event::Data`]): the model decides a
//!   key on the database as it is, as Python reads it live.
//! - **The poll** every [`POLL`] ([`Event::Poll`]).
//! - **Nothing after the end.** Once the model closes ([`Effect::Exit`],
//!   [`Effect::Quit`]) or the operator quits (Ctrl+Q), no event reaches it
//!   again; the loop only waits for the sends in flight (an answer given is
//!   never lost to the exit), then for the pane marks it queued, bounded.
//! - **A frame after every event**, and every
//!   [`FRAME_SECONDS`] while the
//!   empty inbox shows; the toasts drawn over it.
//!
//! [`Effect::Exit`]: inbox_core::popup::Effect::Exit
//! [`Effect::Quit`]: inbox_core::popup::Effect::Quit

mod apply;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender as Tx};
use std::time::Duration;

use inbox_core::empty_inbox::FRAME_SECONDS;
use inbox_core::popup::{Event, Key, Millis, Mode, Open, Popup, Severity, View};
use inbox_core::popup_view::{Ctx, Theme, ViewState, render};

use crate::store::Store;
use crate::tui::back::Back;
use crate::tui::goto::Agents;
use crate::tui::marks::{DRAIN, Marks};
use crate::tui::screen::{Clock, Input, Screen};
use crate::tui::send::Sender;
use crate::tui::snapshot::busy::BusySince;
use crate::tui::snapshot::{Shown, read};
use crate::tui::toasts::{Toast, Toasts};

/// The poll's period (Python `POLL_SECONDS`, 2 s).
pub const POLL: Millis = 2000;
/// While closing, how often the loop looks whether the sends are done.
const CLOSING_TICK: Millis = 50;
const FRAME: Millis = (FRAME_SECONDS * 1000.0) as Millis;

/// What the loop works with.
pub struct Parts {
    /// The popup's connection (migrated at open).
    pub store: Store,
    /// Background sends.
    pub sender: Sender,
    /// herdr's agents, for `g`.
    pub agents: Arc<dyn Agents>,
    /// The pane-mark queue.
    pub marks: Marks,
    /// Where background threads report (a clone goes to each).
    pub back_tx: Tx<Back>,
    /// Their reports.
    pub back_rx: Receiver<Back>,
    /// How long a queued remote action waits (`remote_answer_expiry_hours`).
    pub answer_expiry: Duration,
    /// herdr-radar lacks `?N` (set by a background check).
    pub radar_missing: Arc<AtomicBool>,
    /// The colours.
    pub theme: Theme,
}

/// The loop's state.
struct Ui<'c> {
    popup: Popup,
    parts: Parts,
    toasts: Toasts,
    view: ViewState,
    busy: BusySince,
    clock: &'c dyn Clock,
    closing: bool,
    screen_gone: bool,
}

/// Runs the popup until it closes: opens the model on `open`, then serves
/// `screen` until the model or the operator closes it and every send in
/// flight is recorded.
pub fn run(open: Open, parts: Parts, screen: &mut dyn Screen, clock: &dyn Clock) -> io::Result<()> {
    let busy = BusySince::read(&parts.store, &open.snapshot).unwrap_or_default();
    let (popup, fx) = Popup::open(open, clock.now());
    let mut ui = Ui {
        popup,
        parts,
        toasts: Toasts::default(),
        view: ViewState::default(),
        busy,
        clock,
        closing: false,
        screen_gone: false,
    };
    ui.apply(fx, clock.now());
    let mut next_poll = clock.now() + POLL;
    let mut next_frame = clock.now() + FRAME;
    ui.draw(screen);
    loop {
        ui.reports(clock.now());
        if ui.closing && !ui.parts.sender.in_flight() {
            break;
        }
        let now = clock.now();
        if now >= next_poll {
            ui.poll(now);
            next_poll = now + POLL;
        }
        if now >= next_frame {
            next_frame = now + FRAME;
        }
        ui.toasts.expire(now);
        let mut until = next_poll;
        if ui.popup.mode() == Mode::Empty {
            until = until.min(next_frame);
        }
        if let Some(t) = ui.toasts.next_expiry() {
            until = until.min(t);
        }
        if ui.closing {
            until = until.min(now + CLOSING_TICK);
        }
        let wait = Duration::from_millis(until.saturating_sub(now).max(1));
        match screen.next(wait) {
            Ok(Some(input)) => ui.input(input, clock.now()),
            Ok(None) => {}
            Err(_) => ui.gone(), // the terminal is gone
        }
        ui.reports(clock.now());
        ui.draw(screen);
        if ui.screen_gone {
            break; // nothing to show or read: wait for the sends below, without spinning
        }
    }
    ui.parts.sender.wait();
    ui.parts.marks.drain(DRAIN);
    Ok(())
}

impl Ui<'_> {
    fn input(&mut self, input: Input, now: Millis) {
        if input == Input::Closed {
            return self.gone();
        }
        if self.closing {
            return; // nothing reaches the model after the end
        }
        let in_grace = self.popup.timers().grace_until.is_some_and(|g| now < g);
        match input {
            Input::Key(key) => self.key(key, now),
            // Textual drops these with every other key in the grace window
            Input::Quit if !in_grace => self.closing = true,
            Input::HelpQuit if !in_grace => {
                let mut toast = Toast::new("Press ctrl+q to quit the app", Severity::Information);
                toast.title = Some("Do you want to quit?".into());
                self.toasts.push(toast, now);
            }
            Input::Paste(text) => self.paste(&text, now),
            Input::Quit | Input::HelpQuit | Input::Resize | Input::Closed => {}
        }
    }

    /// The terminal is gone: close, and stop using it.
    fn gone(&mut self) {
        self.closing = true;
        self.screen_gone = true;
    }

    /// A paste (Textual's `Paste`): only an open, focused text box takes it,
    /// and only its first line, as typed characters (Python
    /// `Input._on_paste`'s `splitlines()[0]`); anywhere else it is nothing.
    /// It never presses Enter, and never reaches a binding or a pick.
    fn paste(&mut self, text: &str, now: Millis) {
        let typing = matches!(
            self.popup.view(),
            View::Question(v) if v.editing.is_some() && v.box_focused
        );
        if !typing || self.popup.confirm().is_some() {
            return;
        }
        if let Some(snap) = self.read() {
            let fx = self.popup.handle(Event::Data(snap), now);
            self.apply(fx, now);
        }
        let line = text.lines().next().unwrap_or("");
        for c in line.chars().filter(|c| !c.is_control()) {
            if self.closing {
                return;
            }
            let fx = self.popup.handle(Event::Key(Key::Char(c)), now);
            self.apply(fx, now);
        }
    }

    fn key(&mut self, key: Key, now: Millis) {
        if let Some(snap) = self.read() {
            let fx = self.popup.handle(Event::Data(snap), now);
            self.apply(fx, now);
        }
        if !self.closing {
            let fx = self.popup.handle(Event::Key(key), now);
            self.apply(fx, now);
        }
    }

    fn poll(&mut self, now: Millis) {
        if self.closing {
            return;
        }
        if let Some(snap) = self.read() {
            let fx = self.popup.handle(Event::Poll(snap), now);
            self.apply(fx, now);
        }
    }

    /// What the background threads reported, in order.
    fn reports(&mut self, now: Millis) {
        while let Ok(back) = self.parts.back_rx.try_recv() {
            match back {
                Back::Toast(t) => self.toasts.push(t, now),
                _ if self.closing => {}
                Back::Sent { done, wanted } => {
                    // a failed read: the model's own copy stands in for it
                    let snapshot = self.read().unwrap_or_else(|| self.popup.data().clone());
                    let event = Event::Sent {
                        done,
                        wanted,
                        snapshot,
                    };
                    let fx = self.popup.handle(event, now);
                    self.apply(fx, now);
                }
                Back::Focused { pane, error } => {
                    let fx = self.popup.handle(Event::Focused { pane, error }, now);
                    self.apply(fx, now);
                }
            }
        }
    }

    /// The database for the model now (and the busy times with it); `None`
    /// if it could not be read.
    fn read(&mut self) -> Option<inbox_core::popup::Snapshot> {
        let shown = Shown::of(Some(&self.popup));
        let snap = read(&self.parts.store, &shown, self.clock.wall_time()).ok()?;
        if let Ok(busy) = BusySince::read(&self.parts.store, &snap) {
            self.busy = busy;
        }
        Some(snap)
    }

    fn draw(&mut self, screen: &mut dyn Screen) {
        let wall = self.clock.wall();
        let ctx = Ctx {
            now: self.clock.now(),
            wall: wall as i64,
            busy_for: self.busy.at(wall),
            radar_missing: self.parts.radar_missing.load(Ordering::Relaxed),
            theme: self.parts.theme,
        };
        let (popup, view, toasts) = (&self.popup, &mut self.view, &self.toasts);
        let drawn = screen.draw(&mut |buf| {
            render(popup, &ctx, view, buf);
            toasts.draw(buf);
        });
        if drawn.is_err() {
            self.gone();
        }
    }
}

#[cfg(test)]
mod tests;
