//! Where a key goes, in Textual's order as the Python popup sets it up:
//! every key is stamped, then dropped inside the grace window; Tab/Shift+Tab
//! (priority bindings) switch tabs even under the confirmation; the
//! confirmation takes every other key; the empty view closes on any key but
//! the navigation keys; otherwise the view's own keys, then the app's
//! (←/→ switch tabs, `d` dismiss, `g` go to the agent, Esc back).
//!
//! One deliberate difference: Tab inside an open text box does nothing
//! here. In Textual it moves the focus off the box to the tab row, where
//! no key does anything (`edge.note_box_keys` stops at that Tab).

use super::RELEASE;
use super::entry::{Mode, Tab};
use super::event::{Effect, Severity};
use super::key::Key;
use super::state::{ConfirmTarget, Popup};
use super::views::{Out, View};

impl Popup {
    pub(super) fn key(&mut self, key: &Key) {
        let now = self.now;
        self.timers.prev_key_at = self.timers.last_key_at;
        self.timers.last_key_at = Some(now);
        if self.timers.grace_until.is_some_and(|g| now < g) {
            return; // a view change the operator did not cause: this key was meant for the old one
        }
        if matches!(key, Key::Tab | Key::BackTab) && self.can_switch() {
            return self.switch(if *key == Key::Tab { 1 } else { -1 });
        }
        if let Some(confirm) = self.confirm.take() {
            return self.confirmed(confirm.target, key.is('y'));
        }
        if self.mode == Mode::Empty {
            return match key {
                Key::Left if !self.entries.is_empty() => self.switch(-1),
                Key::Right if !self.entries.is_empty() => self.switch(1),
                _ => self.exit(),
            };
        }
        if self.view_key(key) {
            return;
        }
        match key {
            Key::Left if self.can_switch() => self.switch(-1),
            Key::Right if self.can_switch() => self.switch(1),
            Key::Char('d') if self.can_act(false) => self.dismiss(),
            Key::Char('g') if self.can_act(true) => self.go_to(),
            Key::Escape => self.back(),
            _ => {}
        }
    }

    /// The focused view takes the key (`false`: it does not bind it).
    fn view_key(&mut self, key: &Key) -> bool {
        let armed = !self.held && self.view_armed();
        if matches!(self.view, View::List(_)) {
            return self.list_key(key, armed);
        }
        if let (View::Review(v), Key::Char('x')) = (&self.view, key) {
            if let Some(qid) = v.drop_target() {
                self.write_draft(qid, None);
                if let View::Review(v) = &mut self.view {
                    v.reload(&self.data);
                }
            }
            return true;
        }
        let Popup {
            view,
            fx,
            queue,
            data,
            ..
        } = self;
        let mut out = Out { fx, queue };
        match view {
            View::None | View::List(_) => false,
            View::Question(v) => v.key(key, armed, &mut out),
            View::Review(v) => v.key(key, armed, data, &mut out),
            View::Gone(v) => v.key(key, &mut out),
            View::Failed(v) => v.key(key, armed, &mut out),
            View::Alert(v) => v.key(key, armed, &mut out),
        }
    }

    fn view_armed(&self) -> bool {
        let armed_at = match &self.view {
            View::List(v) => v.armed_at,
            View::Question(v) => v.armed_at,
            View::Review(v) => v.armed_at,
            View::Failed(v) => v.armed_at,
            View::Alert(v) => v.armed_at,
            View::None | View::Gone(_) => 0,
        };
        self.now >= armed_at
    }

    fn editing(&self) -> bool {
        matches!(&self.view, View::Question(v) if v.editing.is_some())
    }

    /// Python `check_action("switch")`.
    fn can_switch(&self) -> bool {
        !self.editing()
            && (self.mode == Mode::Group || (self.mode == Mode::Empty && !self.entries.is_empty()))
    }

    /// Python `check_action("dismiss" | "goto")`.
    fn can_act(&self, goto: bool) -> bool {
        if self.editing() {
            return false;
        }
        match self.mode {
            Mode::List => self
                .list_target()
                .is_some_and(|c| self.entries.contains(&c) && !self.closed.contains(&c)),
            Mode::Group => self
                .current
                .is_some_and(|c| c != Tab::Submit && !self.gone.contains(&c)),
            Mode::Failed | Mode::Alert => goto,
            Mode::Empty => false,
        }
    }

    /// Tab/→ and Shift+Tab/← (Python `action_switch`).
    fn switch(&mut self, delta: isize) {
        if self.held {
            // the switch keys only release a deep link, nothing more
            if delta > 0 && self.now >= self.held_at + RELEASE {
                self.held = false;
            }
            return;
        }
        if self.mode == Mode::Empty {
            let first = self.entries.first().cloned();
            self.show_list(first);
        } else {
            self.navigate(delta, None);
        }
    }

    /// The confirmation closed: `y` confirms.
    fn confirmed(&mut self, target: ConfirmTarget, yes: bool) {
        if !yes {
            return;
        }
        match target {
            ConfirmTarget::Question(qid) => {
                if self.close_question(qid) {
                    self.notify(&format!("Q{qid} dismissed"), Severity::Information);
                    self.gone
                        .insert(Tab::Question(qid), "dismissed".to_string());
                    self.navigate(1, None);
                }
            }
            ConfirmTarget::Group(entry, qids) => {
                let closed: Vec<String> = qids
                    .into_iter()
                    .filter(|&q| self.close_question(q))
                    .map(|q| format!("Q{q}"))
                    .collect();
                if !closed.is_empty() {
                    self.notify(
                        &format!("{} dismissed", closed.join(", ")),
                        Severity::Information,
                    );
                }
                self.closed.insert(entry, "dismissed".to_string());
            }
        }
    }

    /// Dismiss an open question (the effect; the model's copy follows).
    fn close_question(&mut self, qid: i64) -> bool {
        self.fx.push(Effect::Dismiss { qid });
        self.write_status(qid, "dismissed")
    }
}
