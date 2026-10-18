//! Delivery checks shared by the hook paths (Python `service.py`): what a
//! delivery carries, the asker's forward family, and whether it already
//! landed in any transcript of the conversation.

pub mod family;
pub mod landed;
pub mod segments;

pub use family::forward_family;
pub use landed::landed_across;
pub use segments::{REFERENCE_INDEX, expected_segments};
