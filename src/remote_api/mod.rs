//! The remote side of remote machines (Python `remote_api.py`,
//! docs/REMOTE.md): `inbox export` and `inbox apply`, run over ssh on the
//! machine that owns the database, started by the operator's machine with a
//! fixed command line. The request is JSON on stdin and the reply one JSON
//! object on stdout ([`crate::cli::remote`]).
//!
//! - [`keys`]: the database's id, and the keys of its questions and alerts.
//! - [`export`]: what changed since the puller's cursor; no data writes.
//! - [`apply`]: one operator action, recorded in the same transaction.
//!
//! Python's types decide what a request means (a bool is an int, a falsy
//! value is none). Where Python fails with a traceback on a request no inbox
//! sends, this does the closest safe thing, named where it is done.

pub mod apply;
mod detail;
pub mod export;
pub mod keys;

#[cfg(test)]
mod tests;
