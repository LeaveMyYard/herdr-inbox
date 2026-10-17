//! Pure core of herdr-inbox (plan §2.3): `Question`/`Group`/`Alert`,
//! `DeliveryState`, `decide()` and dedup.
//!
//! This crate is the purity boundary: it takes data and the current time and
//! returns data. It does no IO, starts no threads and reads no environment.
//! Empty until the port of the delivery policy starts.
