//! The tools' bodies as the server runs them ([`Tools`]): each call through
//! [`busy_safe`], its store opened as a tool's, its caller resolved per call,
//! its herdr side effects queued ([`Effects`]).
//!
//! Ported so far: `ask_operator` and `check_answers`. The other four answer
//! "not ported yet" until P5-F3.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::Tools;
use super::answers::check_answers;
use super::args::{Ask, Call};
use super::ask::{checks, file};
use super::caller::{self, Env, World};
use super::effects::Effects;
use super::open::{failure, open};
use super::retry::{Backoff, Failure, busy_safe};
use crate::claude::Transcripts;

/// Everything a call needs.
pub struct Runtime {
    pub env: Env,
    pub world: Box<dyn World + Send + Sync>,
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
        let store = open(&self.db)?; // Python opens the store before it resolves the caller
        let caller = caller::resolve(&*self.world, &self.env)?;
        let call = file::Call {
            items: &items,
            context: a.context.as_deref(),
            asked_by: a.asked_by.as_deref(),
            title: a.title.as_deref(),
        };
        let (receipt, effects) =
            file::ask_group(&store, &caller, &call, &self.server, &self.files).map_err(failure)?;
        self.effects.later(effects);
        // `question` gets its one receipt; `questions` the group's
        Ok(if a.questions.as_ref().is_some_and(|q| !q.is_empty()) {
            receipt
        } else {
            file::single(receipt)
        })
    }

    fn check(&self) -> Result<Value, Failure> {
        let caller = caller::resolve(&*self.world, &self.env)?; // Python resolves first here
        let store = open(&self.db)?;
        check_answers(&store, &caller, &self.files).map_err(failure)
    }
}

impl Tools for Runtime {
    fn call(&self, call: Call) -> Result<Value, String> {
        match &call {
            Call::AskOperator(a) => self.safely(|| self.ask(a)),
            Call::CheckAnswers => self.safely(|| self.check()),
            _ => Err("not ported to the Rust server yet (P5-F)".into()),
        }
    }
}

#[cfg(test)]
mod tests;
