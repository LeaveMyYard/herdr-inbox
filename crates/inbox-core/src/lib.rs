//! Pure core of herdr-inbox (plan §2.3): the delivery policy as data in,
//! decisions out.
//!
//! This crate is the purity boundary: it takes data and the current time and
//! returns data. It does no IO, starts no threads and reads no environment,
//! and it knows no agent harness: the callers look things up (the database,
//! herdr, transcripts) and carry the decisions out.
//!
//! - [`hook`]: what the delivery hook does with a session's waiting answers
//!   (Python `hook.deliver`).
//! - [`rewake`]: when the asyncRewake watcher looks, wakes or ends (Python
//!   `rewake.run`).
//!
//! The worker's `decide` (Python `service.deliver_batch`) joins in P5. Every
//! rule here reproduces the hook and rewake rows of the delivery decision
//! table, `tests/fixtures/decisions.json` (`tests/decisions.rs`).

pub mod hook;
pub mod rewake;

/// The index of the long-answer reference among a delivery's expected
/// strings (single message, batch segment, reference; Python
/// `service.REFERENCE_INDEX`): finding only it in a transcript delivers the
/// pointer, not the answer.
pub const REFERENCE_INDEX: usize = 2;
