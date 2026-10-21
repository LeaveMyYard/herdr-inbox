//! What Claude Code's screen says, read from `herdr pane read` (Python
//! `herdr.composer_state`, `herdr.panel_focused`): whether its input box is
//! empty, and whether its background-agents panel has the keyboard. Pure:
//! the caller reads the pane (kit `read_pane`, which already reads line ends
//! as Python's text mode does) and hands the text here.
//!
//! Every rule reproduces `tests/fixtures/herdr-screens.json`, Python's own
//! answers over the screens its tests use (`tests/delivery_screens.rs`).

mod ansi;
mod composer;
mod panel;

pub use composer::composer_state;
pub use panel::panel_focused;

/// Python `str.strip()`: Unicode whitespace off both ends.
fn strip(s: &str) -> &str {
    s.trim_matches(crate::text::py_isspace)
}
