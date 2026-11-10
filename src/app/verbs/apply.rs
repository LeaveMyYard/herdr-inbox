//! One verb's check and write, run inside `BEGIN IMMEDIATE` by the caller.
//! **SAFETY**: in the order of `app-commands-v1.md`, "The state a verb
//! checks": the row exists, for `retry` no live claim, the token. Nothing is
//! written before all three hold, and a stale token is never written past.
//!
//! The popup's `submit` loops over separate transactions, so it is not used
//! here: `write_answer` is the one that joins the caller's.

use serde_json::Value;

use super::args::{Entry, Request, Verb};
use super::closed::{alert_word, failed_gone, group_word, question_word};
use super::detail::normalise;
use super::reply::Code;
use super::route::{Mark, Send, plan, route_of};
use crate::app::feed::text::Text;
use crate::app::feed::{alert, failed, group};
use crate::pyjson::dumps;
use crate::service::answer::{answer_text, stored_answer};
use crate::store::Store;
use crate::store::claims::is_live;
use crate::store::question::Question;
use crate::store::questions::write_answer;

const STALE: &str = "This group changed; look again.";

/// What a transaction came to.
#[derive(Debug)]
pub enum Outcome {
    /// It committed.
    Done(Done),
    /// Nothing changed.
    Refused(Refusal),
}

/// A verb that was done.
#[derive(Debug, Default)]
pub struct Done {
    /// The row's token now, `None` when it is gone.
    pub token: Option<String>,
    /// What changed.
    pub done: Vec<i64>,
    /// Named questions with no draft.
    pub skipped: Vec<i64>,
    /// What waits to be delivered, by route.
    pub send: Vec<Send>,
    /// The panes whose marks moved.
    pub marks: Vec<Mark>,
}

/// A verb that was refused.
#[derive(Debug)]
pub struct Refusal {
    /// The code.
    pub code: Code,
    /// What the mod may show.
    pub message: String,
    /// Why the row is gone.
    pub closed: Option<&'static str>,
}

fn refused(code: Code, message: impl Into<String>) -> rusqlite::Result<Outcome> {
    Ok(Outcome::Refused(Refusal {
        code,
        message: message.into(),
        closed: None,
    }))
}

fn stale(closed: Option<&'static str>) -> rusqlite::Result<Outcome> {
    Ok(Outcome::Refused(Refusal {
        code: Code::Stale,
        message: STALE.to_string(),
        closed,
    }))
}

fn same_token(row: &Value, request: &Request) -> bool {
    row["token"].as_str() == Some(request.token.as_str())
}

/// A write that must take, under the lock, after the checks. It failing means
/// a check was wrong: the transaction rolls back.
fn must(written: bool, what: &str) -> rusqlite::Result<()> {
    if written {
        return Ok(());
    }
    Err(rusqlite::Error::ToSqlConversionFailure(
        format!("{what} did not take").into(),
    ))
}

/// Runs `request`.
pub fn apply(store: &Store, request: &Request) -> rusqlite::Result<Outcome> {
    match request.entry {
        Entry::Group(group) => on_group(store, request, group),
        Entry::Failed(question) => on_failed(store, request, question),
        Entry::Alert(id) => on_alert(store, request, id),
        Entry::Remote => refused(Code::ReadOnly, "Answer this in the herdr popup"),
    }
}

fn on_group(store: &Store, request: &Request, id: i64) -> rusqlite::Result<Outcome> {
    let open = store.group(id, false)?;
    if open.is_empty() {
        return stale(group_word(store, id, &request.questions)?);
    }
    let row = group::entry(store, id, &open, Text::Cut)?;
    if !same_token(&row, request) {
        // the token moved: say so if it was one of these questions that closed
        let mut closed = None;
        for q in &request.questions {
            closed = closed.or(store
                .get(*q)?
                .filter(|q| q.group() == id)
                .and_then(|q| question_word(&q)));
        }
        return stale(closed);
    }
    for q in &request.questions {
        if !open.iter().any(|o| o.id == *q) {
            return refused(
                Code::Invalid,
                format!("Q{q} is not an open question of this group"),
            );
        }
    }
    let mut done = Done::default();
    let unfit = match request.verb {
        Verb::Answer => answer(store, request, &open, &mut done)?,
        Verb::Draft => draft(store, request, &open, &mut done)?,
        _ => dismiss(store, request, &open, &mut done)?,
    };
    if let Some(why) = unfit {
        return refused(Code::Invalid, why);
    }
    let after = store.group(id, false)?;
    if !after.is_empty() {
        done.token = group::entry(store, id, &after, Text::Cut)?["token"]
            .as_str()
            .map(str::to_string);
    }
    Ok(Outcome::Done(done))
}

fn by_id(open: &[Question], id: i64) -> rusqlite::Result<&Question> {
    open.iter()
        .find(|q| q.id == id)
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// `--detail` answers the one open question at once, without it the named
/// drafts go, as the popup's Submit. The refusal if it does not fit.
fn answer(
    store: &Store,
    request: &Request,
    open: &[Question],
    done: &mut Done,
) -> rusqlite::Result<Option<String>> {
    let mut answers: Vec<(i64, String, String)> = Vec::new();
    if let Some(detail) = &request.detail {
        if open.len() > 1 {
            return Ok(Some(
                "This group has several open questions: use draft, then answer".to_string(),
            ));
        }
        let q = by_id(open, request.questions[0])?;
        let detail = match normalise(q, detail) {
            Ok(Some(detail)) => detail,
            Ok(None) => return Ok(Some(format!("Q{} has no answer", q.id))),
            Err(unfit) => return Ok(Some(unfit.0)),
        };
        must(store.set_draft(q.id, Some(&detail))?, "the draft")?;
        answers.push((q.id, answer_text(&detail), dumps(&detail)));
    } else {
        for &id in &request.questions {
            match stored_answer(store, id)? {
                Some((text, json)) => answers.push((id, text, json)),
                None => done.skipped.push(id),
            }
        }
    }
    let mut answered = Vec::new();
    for (id, text, json) in answers {
        must(
            write_answer(store.db(), id, &text, Some(&json))?,
            "the answer",
        )?;
        answered.push(by_id(open, id)?);
        done.done.push(id);
    }
    done.send = plan(store, &answered)?;
    done.marks = Mark::of(store, &answered)?;
    Ok(None)
}

fn draft(
    store: &Store,
    request: &Request,
    open: &[Question],
    done: &mut Done,
) -> rusqlite::Result<Option<String>> {
    let q = by_id(open, request.questions[0])?;
    let detail = match &request.detail {
        None => None,
        Some(detail) => match normalise(q, detail) {
            Ok(detail) => detail,
            Err(unfit) => return Ok(Some(unfit.0)),
        },
    };
    let stored = store.draft_text(q.id)?;
    if stored == detail.as_ref().map(dumps) {
        return Ok(None); // the same again changes nothing
    }
    must(store.set_draft(q.id, detail.as_ref())?, "the draft")?;
    done.done.push(q.id);
    Ok(None)
}

fn dismiss(
    store: &Store,
    request: &Request,
    open: &[Question],
    done: &mut Done,
) -> rusqlite::Result<Option<String>> {
    let chosen: Vec<&Question> = open
        .iter()
        .filter(|q| request.questions.is_empty() || request.questions.contains(&q.id))
        .collect();
    for q in &chosen {
        must(store.set_status(q.id, "dismissed", None)?, "the dismissal")?;
        done.done.push(q.id);
    }
    done.marks = Mark::of(store, &chosen)?;
    Ok(None)
}

fn on_failed(store: &Store, request: &Request, id: i64) -> rusqlite::Result<Outcome> {
    let Some(q) = store.get(id)? else {
        return stale(Some("removed"));
    };
    if let Some(closed) = failed_gone(store, &q)? {
        return stale(closed);
    }
    let claimed = q.delivery_error.as_deref().is_some_and(is_live);
    if request.verb == Verb::Retry && claimed {
        return refused(
            Code::AlreadySending,
            format!("Q{id} is already being delivered"),
        );
    }
    let row = failed::entry(store, &q, false)?;
    if !same_token(&row, request) {
        return stale(None);
    }
    let mut done = Done {
        done: vec![id],
        ..Done::default()
    };
    if request.verb == Verb::Retry {
        store.raise_retry_count(id)?;
        // a stranded row's clock: the next refusal must not strand it again at once
        store.stop_waiting(id)?;
        done.token = failed::entry(store, &q, false)?["token"]
            .as_str()
            .map(str::to_string);
        done.send = vec![route_of(store, &q).map(|route| Send {
            route,
            questions: vec![id],
        })?];
    } else {
        store.hide_failed(id)?;
    }
    Ok(Outcome::Done(done))
}

fn on_alert(store: &Store, request: &Request, id: i64) -> rusqlite::Result<Outcome> {
    let Some(a) = store.alert(id)? else {
        return stale(Some("removed"));
    };
    if a.status != "open" {
        return stale(alert_word(&a));
    }
    if !same_token(&alert::entry(store, &a)?, request) {
        return stale(None);
    }
    must(
        store.acknowledge_alert(id)?.is_some(),
        "the acknowledgement",
    )?;
    Ok(Outcome::Done(Done {
        done: vec![id],
        marks: Mark::of_alert(&a),
        ..Done::default()
    }))
}
