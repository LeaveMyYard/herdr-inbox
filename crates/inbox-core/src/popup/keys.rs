//! Where a key goes, in Textual's order as the Python popup sets it up:
//! every key is stamped, then dropped inside the grace window; Tab/Shift+Tab
//! (priority bindings) switch tabs even under the confirmation; the
//! confirmation takes every other key; the empty view closes on any key but
//! the navigation keys; otherwise the view's own keys, then the app's
//! (←/→ switch tabs, `d` dismiss, `g` go to the agent, Esc back). Which
//! widget has Textual's focus decides who sees the key first ([`Focus`]).

use super::RELEASE;
use super::entry::{Mode, Tab};
use super::focus::Focus;
use super::key::Key;
use super::state::Popup;
use super::views::{Out, View};

impl Popup {
    pub(super) fn key(&mut self, key: &Key) {
        let now = self.now;
        self.timers.prev_key_at = self.timers.last_key_at;
        self.timers.last_key_at = Some(now);
        if self.timers.grace_until.is_some_and(|g| now < g) {
            return; // a view change the operator did not cause: this key was meant for the old one
        }
        let tab = matches!(key, Key::Tab | Key::BackTab);
        if tab && self.can_switch() {
            return self.switch(if *key == Key::Tab { 1 } else { -1 });
        }
        if let Some(confirm) = self.confirm.take() {
            return self.confirmed(confirm.target, key.is('y'));
        }
        if self.mode == Mode::Empty {
            return match key {
                Key::Left if !self.entries.is_empty() => self.switch(-1),
                Key::Right if !self.entries.is_empty() => self.switch(1),
                Key::Escape => {
                    self.exit(); // on_key closes; the app's Esc then quits as well
                    self.quit();
                }
                _ => self.exit(),
            };
        }
        if tab {
            return self.move_focus(*key == Key::Tab); // Textual's focus_next / focus_previous
        }
        if self.focus() != Focus::Chips && self.view_key(key) {
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
            focus,
            ..
        } = self;
        let mut out = Out { fx, queue };
        match view {
            View::None | View::List(_) => false,
            View::Question(v) => {
                let had = (v.editing.is_some(), v.box_focused);
                let taken = v.key(key, armed, &mut out);
                let changed = (v.editing.is_some(), v.box_focused) != had;
                if changed && (v.editing.is_none() || v.box_focused) {
                    *focus = Focus::View; // a box opened or closed: Textual focuses it, or the view
                }
                taken
            }
            View::Review(v) => v.key(key, armed, &mut out),
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
        self.focus() == Focus::Box
    }

    /// Python `check_action("switch")`, and a held remote question: it has
    /// no tabs, but the switch keys must still release its deep link.
    fn can_switch(&self) -> bool {
        let tabs = match self.mode {
            Mode::Group => true,
            Mode::Empty => !self.entries.is_empty(),
            Mode::Remote => self.held,
            _ => false,
        };
        tabs && !self.editing()
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
            Mode::Empty | Mode::Remote => false,
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
}
