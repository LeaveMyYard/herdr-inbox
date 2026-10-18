//! Remote machines (Python `remote.py`, docs/REMOTE.md): the puller's side.
//! [`validate`] checks what a remote's `inbox export` printed (untrusted
//! output) before anything of it reaches the mirror tables
//! ([`crate::store::mirror`]).

pub mod pyvalue;
pub mod validate;
