//! Carrying out the model's effects, in order: toasts, the database writes
//! (on this thread, as Python makes them), sends and `g` handed to their
//! threads, and the end.
//!
//! A write the database refuses ends the batch there (Python's
//! `guarded`: the action stops at the exception): the busy notice shows,
//! and nothing after it runs, so a toast never reports a write that did not
//! happen. The model's own copy may still show the write as made until the
//! next read; that only ever leaves an inert placeholder (a row or tab shown
//! as closed), never a key that acts on the wrong question.
//!
//! A remote write is the one kind the model hears about again: it has
//! announced the action and moved on before the write, and a refusal
//! ([`Outcome::Refused`]) ends the batch ahead of that announcement and hands
//! the model [`Event::RemoteRefused`], which takes the action back.

use std::sync::Arc;

use inbox_core::popup::{Effect, Event, Millis, Severity};

use super::Ui;
use crate::trace;
use crate::tui::goto::go_to;
use crate::tui::send::failure;
use crate::tui::toasts::Toast;
use crate::tui::writes::{Outcome, RemoteEnv, write};

/// The trace step of an effect: what Python's handler of that action is
/// called. Its retry and group dismissal are the same effects here as a
/// submit and a dismissal.
fn step_of(effect: &Effect) -> Option<&'static str> {
    match effect {
        Effect::Draft { .. } => Some("popup.answered"),
        Effect::Send(_) => Some("popup.submit"),
        Effect::Dismiss { .. } => Some("popup.dismiss"),
        _ => None,
    }
}

impl Ui<'_> {
    pub(super) fn apply(&mut self, fx: Vec<Effect>, now: Millis) {
        for effect in fx {
            let _step = step_of(&effect).map(trace::step);
            match effect {
                Effect::Notify { text, severity } => {
                    self.toasts.push(Toast::new(text, severity), now);
                }
                Effect::Send(send) => self.parts.sender.send(send),
                Effect::GoTo { pane, graced } => go_to(
                    pane,
                    graced,
                    Arc::clone(&self.parts.agents),
                    self.parts.back_tx.clone(),
                ),
                Effect::Exit | Effect::Quit => self.closing = true,
                write_fx => match write(
                    &self.parts.store,
                    &write_fx,
                    &self.parts.marks,
                    RemoteEnv {
                        expiry: self.parts.answer_expiry,
                        wall: self.clock.wall_time(),
                    },
                ) {
                    Ok(Outcome::Done) => {}
                    Ok(Outcome::Refused(notice)) => return self.refused(notice, now),
                    Err(err) => {
                        self.toasts
                            .push(Toast::new(failure(&err), Severity::Warning), now);
                        return;
                    }
                },
            }
        }
    }

    /// A remote write was refused: the model takes its action back, on the
    /// database as it is now.
    fn refused(&mut self, notice: Option<String>, now: Millis) {
        let snapshot = self.read();
        let fx = self
            .popup
            .handle(Event::RemoteRefused { notice, snapshot }, now);
        self.apply(fx, now);
    }
}
