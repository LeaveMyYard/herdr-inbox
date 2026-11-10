//! `herdr-inbox app <verb> --json` (`app-commands-v1.md`): how a mod acts on
//! the inbox. **SAFETY**: this decides which answer goes to which question.
//!
//! A verb's token check and its write are one `BEGIN IMMEDIATE` ([`apply`]),
//! so the check says something about the write. After the commit it starts
//! what delivers ([`route`]). Its own process never types: nothing here
//! reaches the delivery pass, and [`route::Effects`] has no way to send.
//!
//! - [`args`], [`detail`]: the command line, refused with no I/O.
//! - [`reply`]: the one JSON object a mod reads.
//! - [`closed`]: why a row is gone.
//! - [`show`]: one question in full, read-only.
//! - [`live`]: the real world behind [`route::Effects`].

pub mod apply;
pub mod args;
pub mod closed;
pub mod detail;
pub mod live;
pub mod reply;
pub mod route;
pub mod show;

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::ErrorCode;

use crate::store::revision::MIGRATION_REVISION;
use crate::store::revision_wanted::{unix_now, want};
use crate::store::{OpenError, Store};
use args::{Parsed, Request, Verb};
use reply::{Code, Reply, Target};
use route::Effects;

/// What a verb works with.
pub struct Ctx<'a> {
    /// The database.
    pub db: &'a Path,
    /// How long it waits for the database, inside the 5 s budget.
    pub db_wait: Duration,
    /// Core delivery exists (B3). Until it does every core answer is `held`.
    pub core_delivery: bool,
    /// What starts after a commit.
    pub fx: &'a mut dyn Effects,
}

/// Runs the words after `app`.
pub fn run(argv: &[String], ctx: &mut Ctx<'_>) -> Reply {
    match args::parse(argv) {
        Parsed::Help => Reply::help(),
        Parsed::Usage(why) => Reply::usage(&why),
        Parsed::Show { question, json } => show::run(question, json, ctx),
        Parsed::Run(request) => verb(&request, ctx),
    }
}

fn verb(request: &Request, ctx: &mut Ctx<'_>) -> Reply {
    let target = Target::of(request);
    if let Some(why) = request.not_for_this_row() {
        return target.refuse(Code::ReadOnly, why, None);
    }
    let started = Instant::now();
    let store = match open(ctx) {
        Ok(store) => store,
        Err(refusal) => return refusal.at(&target),
    };
    // what opening took comes out of the same wait
    let wait = ctx.db_wait.saturating_sub(started.elapsed());
    if let Err(e) = store.db().busy_timeout(wait) {
        return failed(&e).at(&target);
    }
    let result = store.immediate(|_| apply::apply(&store, request));
    let done = match result {
        Ok(apply::Outcome::Done(done)) => done,
        Ok(apply::Outcome::Refused(no)) => return target.refuse(no.code, &no.message, no.closed),
        Err(e) => return failed(&e).at(&target),
    };
    let delivery = route::deliver(
        &done.send,
        request.verb == Verb::Retry,
        ctx.core_delivery,
        ctx.fx,
    );
    ctx.fx.refresh_marks(&done.marks);
    target.done(&done, delivery)
}

/// A refusal before any row was looked at.
pub struct Early {
    code: Code,
    message: String,
}

impl Early {
    fn at(&self, target: &Target) -> Reply {
        target.refuse(self.code, &self.message, None)
    }
}

/// The database for a verb, with its wait. `not_migrated` starts the worker
/// that migrates it.
fn open(ctx: &mut Ctx<'_>) -> Result<Store, Early> {
    let not_ready = |ctx: &mut Ctx<'_>| {
        let _ = ctx.fx.ensure_worker(None);
        Early {
            code: Code::NotMigrated,
            message: "The inbox is being upgraded; try again in a few seconds".to_string(),
        }
    };
    match Store::open(ctx.db, ctx.db_wait) {
        Ok(store) => Ok(store),
        Err(OpenError::NotReady(_)) => Err(not_ready(ctx)),
        Err(OpenError::Sqlite(_)) if !ctx.db.exists() => {
            want(ctx.db, MIGRATION_REVISION, unix_now());
            Err(not_ready(ctx))
        }
        Err(OpenError::Sqlite(e)) => Err(failed(&e)),
    }
}

/// A failed statement as a refusal: nothing was changed.
fn failed(e: &rusqlite::Error) -> Early {
    if matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    ) {
        return Early {
            code: Code::Busy,
            message: "The inbox database is busy; try again in a moment".to_string(),
        };
    }
    // SQLite's own words; rusqlite adds the path, which is nobody's business
    let text = e.to_string();
    let text = text.split(": ").next().unwrap_or_default();
    Early {
        code: Code::Error,
        message: format!("The inbox database failed: {text}"),
    }
}
