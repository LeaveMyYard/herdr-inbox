//! The inbox's marks on herdr (Python `herdr.set_open_count`,
//! `service.update_markers`): the pane's `$inbox_q` token and the `❓N`
//! prefix of the tab it is in now, plus the notifications.
//!
//! - [`Surface`]: the herdr calls these need, each one as Python's
//!   `herdr.py` makes it and never failing the caller (a failure is a
//!   `None`, an `Err` text, or nothing). [`live::Live`] makes them over the
//!   kit's client; the tests script them.
//! - [`popup`]: where the popup opens from (the plugin's root, the pane
//!   that was focused).
//! - [`markers`]: one pane's token and its tab's prefix
//!   ([`markers::Markers::update`]).

pub mod live;
pub mod markers;
pub mod popup;

/// The pane token's source and key (Python `herdr.SOURCE`, `herdr.TOKEN`).
pub const SOURCE: &str = "herdr-inbox";
/// The pane token herdr-radar shows (`$inbox_q`).
pub const TOKEN: &str = "inbox_q";

/// The herdr side of the inbox's marks.
pub trait Surface {
    /// The pane's `inbox_q` token: `?N` for `count > 0`, else cleared
    /// (Python `herdr.set_open_count`).
    fn set_open_count(&mut self, pane: &str, count: i64);
    /// `(pane, tab)` for every pane, in herdr's order; `None` if herdr is
    /// unreachable (Python `herdr.pane_tabs`).
    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>>;
    /// The tab's label now, if herdr says (Python `herdr.tab_label`).
    fn tab_label(&mut self, tab: &str) -> Option<String>;
    /// Renames the tab; `Err` is herdr's error (Python `herdr.rename_tab`).
    fn rename_tab(&mut self, tab: &str, label: &str) -> Result<(), String>;
    /// A desktop notification with the inbox's sound (Python `herdr.notify`).
    fn notify(&mut self, title: &str, body: &str);
    /// Opens the inbox's popup with `env` added to its command's environment
    /// (Python `herdr.open_popup`); `Err` is herdr's error.
    fn open_popup(&mut self, env: &[(String, String)]) -> Result<(), String>;
}
