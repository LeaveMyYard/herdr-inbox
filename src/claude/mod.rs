//! The Claude Code lookups the inbox's hooks need (Python `transcript.py`),
//! Claude-side and plugin-side, on top of the kit's `agent_session::claude`
//! (plan §2.3, "Harness-neutral seams": delivery stays plugin-side).

pub mod chain;
pub mod continued;
pub mod edges;
pub mod find;
pub mod landed;

pub use chain::chain;
pub use continued::continued_in;
pub use edges::{Edge, continued_edges};
pub use find::Transcripts;
