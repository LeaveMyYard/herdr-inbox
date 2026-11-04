//! What a delivery does before its pass (Python `deliver_batch`'s preamble):
//! a fresh send makes sure the worker runs (it retries whatever the send
//! leaves pending), and an answer asked on another herdr server is left to
//! that server's worker, which is made sure of too.

use crate::store::{Question, Store};

/// The answers among `qs` this process delivers: those owned by no server
/// or by `mine`. `ensure` is asked to start the worker: for this server
/// first unless `retry` (the worker's own retries), then once per other
/// server that owns one of `qs`, in the order they first appear.
pub fn local_answers(
    store: &Store,
    qs: &[Question],
    retry: bool,
    mine: &str,
    ensure: &mut dyn FnMut(Option<&str>),
) -> rusqlite::Result<Vec<Question>> {
    if !retry {
        ensure(None);
    }
    let mut local = Vec::new();
    let mut foreign: Vec<String> = Vec::new();
    for q in qs {
        match store.server_of(q.id)? {
            Some(server) if server != mine => {
                if !foreign.contains(&server) {
                    foreign.push(server);
                }
            }
            _ => local.push(q.clone()),
        }
    }
    for server in &foreign {
        ensure(Some(server));
    }
    Ok(local)
}

#[cfg(test)]
mod tests;
