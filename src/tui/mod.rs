//! The inbox popup's IO (`herdr-inbox tui`, plan herdr-app-inbox §2.3 P6-D;
//! Python `tui.InboxApp`'s IO): the pure model ([`inbox_core::popup`]) and
//! renderer ([`inbox_core::popup_view`]) wired to the database, herdr, the
//! terminal and the clock. **SAFETY**: this carries out the model's sends.
//!
//! Each module does one thing:
//!
//! - [`run`]: the loop, events in and effects out, a frame after each;
//! - [`snapshot`]: the database as the model reads it;
//! - [`writes`]: the model's database writes (drafts, dismissals, hides,
//!   acknowledgements, a remote machine's queued actions);
//! - [`send`]: background sends (recorded, then handed to delivery), and
//!   closing only once they are recorded;
//! - [`goto`]: `g`, going to the asking agent;
//! - [`open`]: the operator's pane and the deep link the popup opens on;
//! - [`marks`]: the panes' marks, refreshed on a background thread;
//! - [`toasts`]: the notices, their lifetime and their cells;
//! - [`back`]: what background threads report;
//! - [`screen`], [`keys`], [`term`]: the terminal, its keys and the clock;
//! - [`live`]: the real things behind all of it.

pub mod back;
pub mod goto;
pub mod keys;
pub mod live;
pub mod marks;
#[cfg(test)]
mod mirror_rows;
pub mod open;
pub mod run;
pub mod screen;
pub mod send;
pub mod snapshot;
pub mod term;
pub mod toasts;
pub mod writes;
