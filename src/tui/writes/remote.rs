//! The model's writes on the remote machines' mirror, local database writes
//! only (Python `remote.enqueue`, `enqueue_alert_ack`, `_settle`, `mark_seen`,
//! `clear_problem`): the delivery worker talks to the machine.
//!
//! An answer or a dismissal goes under the identity the operator saw, once.
//! An action the mirror cannot take under it is refused, and so is a second
//! action on a question that has one (the model's copy can be behind: only a
//! read tells it what is queued, and a read can fail). So is any write the
//! database fails. The model has moved on by then, so every way this does not
//! write ends in [`Outcome::Refused`].

use std::time::{Duration, SystemTime};

use inbox_core::popup::{Effect, RemoteRef};
use serde_json::{Map, Value, json};

use super::Outcome;
use crate::store::Store;
use crate::store::mirror::Gone;
use crate::token::request_id;
use crate::tui::send::failure;
use crate::tui::snapshot::convert::detail_json;

/// Why a second action on a question is refused.
const ALREADY_QUEUED: &str = "that question already has an action queued";

/// What a remote write needs besides the database.
#[derive(Clone, Copy, Debug)]
pub struct RemoteEnv {
    /// How long a queued action waits (`remote_answer_expiry_hours`).
    pub expiry: Duration,
    /// The time now.
    pub wall: SystemTime,
}

/// Makes `effect`'s write, if it is one of these.
pub(super) fn write(store: &Store, effect: &Effect, env: RemoteEnv) -> Outcome {
    let done = match effect {
        Effect::RemoteAnswer { question, detail } => {
            queue(store, question, "answer", Some(&detail_json(detail)), env)
        }
        Effect::RemoteDismiss { question } => queue(store, question, "dismiss", None, env),
        Effect::RemoteAck {
            machine_id,
            alert,
            db_id,
            akey,
        } => {
            let shown = shown(*alert, "akey", akey, db_id);
            ack(store, machine_id, &shown, env)
        }
        Effect::ClearOutbox { request_id, expire } => store
            .clear_outbox(request_id, *expire, &mut || env.wall)
            .map(|cleared| {
                if cleared {
                    Outcome::Done
                } else {
                    Outcome::Refused(None) // being sent by now: as Python's `d`, nothing is said
                }
            }),
        Effect::ClearProblem {
            machine_id,
            remote_id,
        } => store
            .clear_problem(machine_id, *remote_id, env.wall)
            .map(|()| Outcome::Done),
        _ => Ok(Outcome::Done),
    };
    done.unwrap_or_else(|err| Outcome::Refused(Some(failure(&err))))
}

/// The identity shown, as the mirror's writes read it (Python's `data`).
fn shown(id: i64, key_name: &str, key: &str, db_id: &str) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("id".into(), json!(id));
    data.insert(key_name.into(), json!(key));
    data.insert("_db_id".into(), json!(db_id));
    data
}

fn queue(
    store: &Store,
    question: &RemoteRef,
    action: &str,
    detail: Option<&Value>,
    env: RemoteEnv,
) -> rusqlite::Result<Outcome> {
    let data = shown(question.remote_id, "qkey", &question.qkey, &question.db_id);
    let queued = store.immediate(|_| {
        let RemoteRef {
            machine_id,
            remote_id,
            db_id,
            qkey,
        } = question;
        if store.action_queued(machine_id, *remote_id, db_id, qkey)? {
            return Ok(Err(Gone(ALREADY_QUEUED.to_string())));
        }
        store.enqueue(
            machine_id,
            &data,
            action,
            detail,
            &request_id(),
            &mut || env.wall,
            env.expiry,
        )
    })?;
    Ok(match queued {
        Ok(_) => Outcome::Done,
        Err(Gone(why)) => Outcome::Refused(Some(format!("not queued: {why}"))),
    })
}

/// The outbox row and the mirror's `acked` together (Python's `transaction`).
fn ack(
    store: &Store,
    machine_id: &str,
    data: &Map<String, Value>,
    env: RemoteEnv,
) -> rusqlite::Result<Outcome> {
    let queued = store.immediate(|_| {
        store.enqueue_alert_ack(
            machine_id,
            data,
            &request_id(),
            &mut || env.wall,
            env.expiry,
        )
    })?;
    Ok(match queued {
        Ok(_) => Outcome::Done,
        Err(Gone(why)) => Outcome::Refused(Some(why)),
    })
}

#[cfg(test)]
mod tests;
