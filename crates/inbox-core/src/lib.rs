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
//! - [`verify`]: what a typed answer's landing records, and whether a
//!   hook-sent answer arrived (Python `_record_outcome`,
//!   `verify_hook_deliveries`).
//!
//! - [`deliver`]: what the delivery worker does with a pane's waiting answers
//!   (Python `service.deliver_batch`).
//!
//! Every rule here reproduces the delivery decision table,
//! `tests/fixtures/decisions.json` (`tests/decisions.rs`, `tests/deliver_rows.rs`,
//! `tests/verify_rows.rs`).

pub mod deliver;
pub mod hook;
pub mod rewake;
pub mod verify;

/// The index of the long-answer reference among a delivery's expected
/// strings (single message, batch segment, reference; Python
/// `service.REFERENCE_INDEX`): finding only it in a transcript delivers the
/// pointer, not the answer.
pub const REFERENCE_INDEX: usize = 2;
