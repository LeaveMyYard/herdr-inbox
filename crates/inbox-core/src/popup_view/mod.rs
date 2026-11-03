//! The inbox popup's renderer (plan herdr-app-inbox §2.3 P6-B): a
//! [`Popup`]'s state in, ratatui cells out. It draws what Python's Textual
//! popup (`tui.InboxApp`) draws, cell for cell (char, colours, bold): the
//! group list, a group's question tabs and its Submit tab, the question
//! view with its options, notes, text box and previews, ⚠ undelivered
//! answers, alerts, placeholders and the dismiss confirmation, in Claude
//! Code's theme roles ([`theme`]). The app-screen goldens
//! (`tests/fixtures/screens/*.json`, `tests/popup_screens.rs`) hold it to
//! that. The empty popup is design 2C ([`crate::empty_inbox`], Q179).
//!
//! Pure: it reads only the popup's accessors and the [`Ctx`] the caller
//! passes (the clocks, the busy times the database knows); it does no IO.
//! [`ViewState`] is its memory between frames, as Textual's widgets keep
//! theirs: how far the tab row and the scrolling body are scrolled.

mod alerts;
mod canvas;
mod chips;
mod confirm;
mod empty;
mod fmt;
mod input;
mod iso;
mod layout;
mod list;
mod marks;
mod question;
mod review;
mod screen;
mod scroll;
mod small;
mod text;
pub mod theme;
mod wrap;

use std::collections::HashMap;

use ratatui_core::buffer::Buffer;

use crate::popup::{Millis, Popup};
pub use theme::Theme;

/// What the renderer needs besides the popup.
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// The popup's clock (the one its events carry): the empty inbox's
    /// animation runs from the view's last change.
    pub now: Millis,
    /// Wall-clock time, Unix seconds: the ages shown (`56m`, `1d`).
    pub wall: i64,
    /// Seconds each pending answer has waited (Python `service._busy_for`),
    /// by question id: "agent busy — will send in N s".
    pub busy_for: HashMap<i64, f64>,
    /// herdr-radar is installed but its sidebar lacks `?N` (Python
    /// `check_radar`): the hint under the view.
    pub radar_missing: bool,
    /// The colours.
    pub theme: Theme,
}

/// The renderer's memory between frames: scroll positions, as Textual's
/// containers keep them. Keep one per popup; start from `default()`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    /// The tab row's sideways scroll (one row for the popup's life).
    chips_x: u16,
    /// The scrolling body's offset, and the view it belongs to (the time
    /// it was shown): a new view starts at the top.
    body: (Option<Millis>, u16),
}

/// Draw `popup` into `buf` (its whole area), updating `state`.
pub fn render(popup: &Popup, ctx: &Ctx, state: &mut ViewState, buf: &mut Buffer) {
    screen::render(popup, ctx, state, buf);
}
