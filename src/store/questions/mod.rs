//! The question reads and writes of the inbox (Python `Store`): asking, the
//! operator's drafts and answers, closing, and the owner queries an agent's
//! tools use. The delivery state lives in `claims` and the hook paths.

mod owner;
mod reads;
mod writes;

pub use owner::Owner;
pub use writes::NewQuestion;

/// An answered question is handed back to a repeat only this long (Python
/// `ANSWER_REUSE`): later the same words are probably a new instance.
pub const ANSWER_REUSE: std::time::Duration = std::time::Duration::from_secs(3600);

/// The `delivery_error` prefix of an answer waiting to be sent (Python `PENDING`).
pub const PENDING: &str = "pending: ";
