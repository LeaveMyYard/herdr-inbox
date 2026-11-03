//! Which widget has the keys, as Textual's focus moves in the Python popup.
//! It matters in one place: Tab or Shift+Tab inside an open text box moves
//! the focus off the box while the box stays open, and then the keys go to
//! the view or to the app again (`edge.tab_in_a_box_*` in the golden).

use super::entry::Mode;
use super::state::Popup;
use super::views::View;

/// Where the keys go first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The view on screen.
    View,
    /// The open text box: it takes every key it edits with.
    Box,
    /// The tab row: only the app's keys work (←/→, Tab, `d`, `g`, Esc). It
    /// also holds the focus in the empty view.
    Chips,
    /// The view's scrolling option list: keys reach the view as with
    /// [`Focus::View`]. (In Textual, ↑/↓ there scroll instead when the list
    /// overflows the popup; the model does not know the popup's size.)
    Choices,
}

impl Focus {
    /// The golden's name.
    pub fn name(self) -> &'static str {
        match self {
            Focus::View => "view",
            Focus::Box => "box",
            Focus::Chips => "chips",
            Focus::Choices => "choices",
        }
    }
}

impl Popup {
    /// Where the keys go first.
    pub fn focus(&self) -> Focus {
        match &self.view {
            View::None => Focus::Chips,
            View::Question(v) if v.editing.is_some() && v.box_focused => Focus::Box,
            _ => self.focus,
        }
    }

    /// Tab (`next`) or Shift+Tab where they switch nothing: Textual moves the
    /// focus to the next focusable widget.
    pub(super) fn move_focus(&mut self, next: bool) {
        let chips_shown = self.mode == Mode::Group && self.all_tabs().len() > 1;
        match &mut self.view {
            View::Question(v) if v.editing.is_some() && v.box_focused => {
                v.box_focused = false;
                self.focus = match (next, chips_shown) {
                    (true, true) => Focus::Chips,
                    (true, false) => Focus::View,
                    (false, _) => Focus::Choices,
                };
            }
            View::List(_) => {
                self.focus = if self.focus == Focus::View {
                    Focus::Choices
                } else {
                    Focus::View
                };
            }
            _ => {}
        }
    }
}
