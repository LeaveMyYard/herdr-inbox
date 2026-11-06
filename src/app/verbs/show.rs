//! `app show --question Q [--json]`: one question in full, the way out for a
//! question the feed cut or left out. One read transaction, no token, no
//! write. The row's token is the published one, so the other verbs take it,
//! and it has no more power than one from the feed: a verb still checks it
//! against the database.

use std::fmt::Write;

use serde_json::{Value, json};

use super::closed::question_word;
use super::reply::{Code, Reply, Target};
use super::{Ctx, failed};
use crate::app::feed::group;
use crate::app::feed::text::Text;
use crate::store::revision::MIGRATION_REVISION;
use crate::store::revision_wanted::{unix_now, want};
use crate::store::{OpenError, Store};

/// `exec` reads no more of a command's output than this.
pub const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// Runs `show`.
pub fn run(question: i64, json: bool, ctx: &mut Ctx<'_>) -> Reply {
    let reply = in_json(question, ctx);
    if !json {
        return as_text(reply);
    }
    if reply.stdout.len() > MAX_OUTPUT {
        let why = "This question is too large to show here: run this command in a terminal, without --json";
        let row: Value = serde_json::from_str(&reply.stdout).unwrap_or_default();
        let entry = row["entry"].as_str().unwrap_or_default();
        return Target::new("show", entry).refuse(Code::Error, why, None);
    }
    reply
}

fn in_json(question: i64, ctx: &mut Ctx<'_>) -> Reply {
    let unknown = Target::new("show", &format!("group:{question}"));
    let store = match Store::open_read_only(ctx.db, ctx.db_wait) {
        Ok(store) => store,
        Err(OpenError::NotReady(_)) => return not_ready(ctx, &unknown),
        Err(OpenError::Sqlite(_)) if !ctx.db.exists() => return not_ready(ctx, &unknown),
        Err(OpenError::Sqlite(e)) => return failed(&e).at(&unknown),
    };
    match read(&store, question, || {}) {
        Ok(reply) => reply,
        Err(e) => failed(&e).at(&unknown),
    }
}

fn not_ready(ctx: &mut Ctx<'_>, target: &Target) -> Reply {
    want(ctx.db, MIGRATION_REVISION, unix_now()); // a worker holding the lock hands over
    let _ = ctx.fx.ensure_worker(None);
    target.refuse(
        Code::NotMigrated,
        "The inbox is being upgraded; try again in a few seconds",
        None,
    )
}

/// The row, with `between` run after the question is read and before its group.
fn read(store: &Store, question: i64, between: impl FnOnce()) -> rusqlite::Result<Reply> {
    // rolled back on drop: the row and its token come from one moment
    let tx = store.db().unchecked_transaction()?;
    let stale = "This group changed; look again.";
    let Some(q) = store.get(question)? else {
        let target = Target::new("show", &format!("group:{question}"));
        return Ok(target.refuse(Code::Stale, stale, Some("removed")));
    };
    let key = format!("group:{}", q.group());
    let target = Target::new("show", &key);
    if q.status != "open" {
        return Ok(target.refuse(Code::Stale, stale, question_word(&q)));
    }
    between();
    let open = store.group(q.group(), false)?;
    let row = group::entry(store, q.group(), &open, Text::Whole)?;
    tx.commit()?;
    Ok(Reply::json(
        &json!({"v": 1, "ok": true, "verb": "show", "entry": key, "row": row}),
    ))
}

#[cfg(test)]
mod tests;

/// The same for a terminal: the group's questions as plain text.
fn as_text(reply: Reply) -> Reply {
    let Ok(value) = serde_json::from_str::<Value>(&reply.stdout) else {
        return reply;
    };
    if value["ok"] != true {
        let why = value["message"].as_str().unwrap_or_default();
        return Reply {
            exit: reply.exit,
            stdout: String::new(),
            stderr: format!("{why}\n"),
        };
    }
    let mut text = String::new();
    for q in value["row"]["questions"].as_array().into_iter().flatten() {
        let _ = writeln!(
            text,
            "Q{}: {}",
            q["id"],
            q["question"].as_str().unwrap_or_default()
        );
        if let Some(context) = q["context"].as_str() {
            let _ = writeln!(text, "\n{context}");
        }
        for o in q["options"].as_array().into_iter().flatten() {
            let _ = writeln!(text, "  - {}", o["label"].as_str().unwrap_or_default());
            if let Some(d) = o["description"].as_str() {
                let _ = writeln!(text, "    {d}");
            }
        }
    }
    Reply {
        exit: 0,
        stdout: text,
        stderr: String::new(),
    }
}
