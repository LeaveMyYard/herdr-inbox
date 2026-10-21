//! The store module against real SQLite databases built from the captured
//! schema (tests/fixtures/schema.json).

mod alerts_golden;
mod atomic;
mod candidates;
mod claims;
mod common;
mod delivery_golden;
mod family;
mod golden;
mod liveness;
mod migrate;
mod mirror_golden;
mod outbox_golden;
mod pending;
mod questions_golden;
mod readiness;
mod sessions;
