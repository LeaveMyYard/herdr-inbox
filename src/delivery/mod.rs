//! Delivery checks shared by the hook paths (Python `service.py`): what a
//! delivery carries, the asker's forward family, whether it already landed
//! in any transcript of the conversation, what Claude Code's screen says
//! ([`screen`]) and the pane's herdr calls ([`pane`]).

pub mod family;
pub mod landed;
pub mod messages;
pub mod pane;
pub mod plans;
pub mod run;
pub mod screen;
pub mod segments;
pub mod watch;

pub use family::forward_family;
pub use landed::landed_across;
pub use segments::{REFERENCE_INDEX, expected_segments};
