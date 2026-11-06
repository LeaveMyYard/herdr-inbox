//! The tools' bodies as the server runs them ([`Tools`]): each call through
//! [`busy_safe`], its store opened as a tool's, its caller resolved per call,
//! its herdr side effects queued ([`Effects`]).
//!
//! The order of the store open and the caller's resolution is Python's:
//! `ask_operator`, `alert_operator` and `withdraw_alert` open the store first;
//! `check_answers`, `list_my_questions` and `withdraw_question` resolve first.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::Tools;
use super::alerting::{self, Raise};
use super::answers::check_answers;
use super::args::{Ask, Call};
use super::ask::{checks, file};
use super::caller::{Caller, Env};
use super::effects::Effects;
use super::mine;
use super::open::{failure, open};
use super::retry::{Backoff, Failure, busy_safe};
use crate::claude::Transcripts;
use crate::host::{self, Host};
use crate::store::Store;
use crate::store::alerts::Effect;
use crate::trace;

/// Everything a call needs.
pub struct Runtime {
    pub env: Env,
    /// Where the caller may run, in the order they are asked (`crate::host`).
    pub hosts: Vec<Box<dyn Host>>,
    pub files: Transcripts,
    /// The inbox database.
    pub db: PathBuf,
    /// The herdr server this process talks to (its socket): only its worker types answers.
    pub server: String,
    pub backoff: Backoff,
    pub effects: Arc<Effects>,
    /// Starts the delivery worker unless one runs (Python `worker.ensure`).
    pub ensure_worker: Box<dyn Fn() + Send + Sync>,
    pub sleep: fn(Duration),
}

impl Runtime {
    fn safely(&self, attempt: impl FnMut() -> Result<Value, Failure>) -> Result<Value, String> {
        busy_safe(attempt, &self.backoff, &*self.ensure_worker, &|d| {
            (self.sleep)(d)
        })
    }

    fn ask(&self, a: &Ask) -> Result<Value, Failure> {
        let items = match checks::items(a) {
            Ok(items) => items,
            Err(receipt) => return Ok(receipt),
        };
        let store = trace::timed("mcp.ask_operator", || {
            trace::timed("mcp.tool_store", || open(&self.db, self.backoff.attempt))
        })?;
        let _body = trace::step("mcp.ask_operator.body");
        let caller = self.caller()?;
        let call = file::Call {
            items: &items,
            context: a.context.as_deref(),
            asked_by: a.asked_by.as_deref(),
            title: a.title.as_deref(),
        };
        let filed = file::ask_group(&store, &caller, &call, &self.server, &self.files);
        // Python's step is under `if new`: only a new question has effects
        let new = matches!(&filed, Ok((_, effects)) if !effects.is_empty());
        let _markers = new.then(|| trace::step("ask.markers"));
        let receipt = self.with_effects(filed)?;
        // `question` gets its one receipt; `questions` the group's
        Ok(if a.questions.as_ref().is_some_and(|q| !q.is_empty()) {
            receipt
        } else {
            file::single(receipt)
        })
    }

    /// Who is asking, now.
    fn caller(&self) -> Result<Caller, Failure> {
        host::caller(&self.hosts, &self.env)
    }

    /// The store, then the caller (Python `with tool_store() as store: ...(resolve_caller())`).
    fn store_then_caller(&self) -> Result<(Store, Caller), Failure> {
        let store = open(&self.db, self.backoff.attempt)?;
        Ok((store, self.caller()?))
    }

    /// The caller, then the store (Python `caller = resolve_caller(); with tool_store()`).
    fn caller_then_store(&self) -> Result<(Store, Caller), Failure> {
        let caller = self.caller()?;
        Ok((open(&self.db, self.backoff.attempt)?, caller))
    }

    /// A body's receipt, its effects queued.
    fn with_effects(&self, done: rusqlite::Result<(Value, Vec<Effect>)>) -> Result<Value, Failure> {
        let (receipt, effects) = done.map_err(failure)?;
        self.effects.later(effects);
        Ok(receipt)
    }

    fn run(&self, call: &Call) -> Result<Value, Failure> {
        let files = &self.files;
        match call {
            Call::AskOperator(a) => self.ask(a),
            Call::CheckAnswers => {
                let (store, c) = self.caller_then_store()?;
                check_answers(&store, &c, files).map_err(failure)
            }
            Call::ListMyQuestions { include_closed } => {
                let (store, c) = self.caller_then_store()?;
                mine::list_my_questions(&store, &c, *include_closed, files).map_err(failure)
            }
            Call::WithdrawQuestion { id, reason } => {
                let (store, c) = self.caller_then_store()?;
                self.with_effects(mine::withdraw_question(&store, &c, *id, reason, files))
            }
            Call::AlertOperator {
                title,
                message,
                severity,
                asked_by,
            } => {
                let (store, c) = self.store_then_caller()?;
                let raise = Raise {
                    title,
                    message: message.as_deref(),
                    severity,
                    asked_by: asked_by.as_deref(),
                };
                self.with_effects(alerting::alert_operator(
                    &store,
                    &c,
                    &raise,
                    &self.server,
                    files,
                ))
            }
            Call::WithdrawAlert { id } => {
                let (store, c) = self.store_then_caller()?;
                self.with_effects(alerting::withdraw_alert(&store, &c, *id, files))
            }
        }
    }
}

impl Tools for Runtime {
    fn call(&self, call: Call) -> Result<Value, String> {
        self.safely(|| self.run(&call))
    }
}

#[cfg(test)]
mod tests;
