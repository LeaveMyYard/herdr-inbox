//! The whole feed from one read transaction (`app-feed-v1.md`, "Feed"):
//! `entries` and `new_since` come from the same moment, so a mark never runs
//! ahead of its row.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::fit::{HEAD_ROOM, Item, MAX_FEED, Need, fit, left_out};
use super::text::Text;
use super::{alert, failed, group, remote};
use crate::store::Store;
use crate::store::question::Question;
use crate::worker::duties::served::CAPS;

/// A feed that says nothing about the inbox: `ok` false, with why.
pub fn unavailable(wall: SystemTime, why: &str) -> Value {
    feed(
        wall,
        false,
        vec![why.to_string()],
        omitted_none(),
        (0, 0),
        Vec::new(),
    )
}

/// The feed of `store` at wall-clock time `wall`.
pub fn read(store: &Store, wall: SystemTime) -> rusqlite::Result<Value> {
    read_around(store, wall, || {})
}

/// [`read`], with `between` run after the rows and before the marks.
fn read_around(store: &Store, wall: SystemTime, between: impl FnOnce()) -> rusqlite::Result<Value> {
    // rolled back on drop, so a failed read never leaves the connection in a transaction
    let tx = store.db().unchecked_transaction()?;
    let snapshot = read_now(store, wall, between)?;
    tx.commit()?;
    Ok(snapshot)
}

fn read_now(store: &Store, wall: SystemTime, between: impl FnOnce()) -> rusqlite::Result<Value> {
    let mut items: Vec<Item> = Vec::new();
    let mut errors = Vec::new();
    let remote = remote::read(store, wall).unwrap_or_else(|_| remote::Rows {
        unreadable: 1,
        ..remote::Rows::default()
    });
    if remote.unreadable > 0 {
        errors.push("remote machines could not be read".to_string());
    }
    for a in store.open_alerts()? {
        items.push(Item {
            entry: alert::entry(store, &a)?,
            need: Need::Alert,
        });
    }
    items.extend(remote.alerts.into_iter().map(remote_item));
    for (id, open) in groups(store)? {
        let ids = open.iter().map(|q| q.id).collect();
        items.push(Item {
            entry: group::entry(store, id, &open, Text::Cut)?,
            need: Need::Group(ids),
        });
    }
    let retried = retried(store)?;
    for q in store.failed_deliveries()? {
        items.push(Item {
            entry: failed::entry(store, &q, retried.contains(&q.id))?,
            need: Need::Failed,
        });
    }
    let rest = [remote.groups, remote.outbox, remote.problems];
    items.extend(rest.into_iter().flatten().map(remote_item));

    let (entries, omitted) = fit(items, MAX_FEED - HEAD_ROOM);
    match left_out(&omitted) {
        0 => {}
        1 => errors.push("1 entry left out to stay under 1 MiB".to_string()),
        n => errors.push(format!("{n} entries left out to stay under 1 MiB")),
    }
    between();
    Ok(feed(wall, true, errors, omitted, marks(store)?, entries))
}

fn remote_item(entry: Value) -> Item {
    Item {
        entry,
        need: Need::Remote,
    }
}

/// The open questions by group, the groups by their first open question.
fn groups(store: &Store) -> rusqlite::Result<Vec<(i64, Vec<Question>)>> {
    let mut open = store.list(false)?;
    open.sort_by_key(|q| q.id);
    let mut out: Vec<(i64, Vec<Question>)> = Vec::new();
    for q in open {
        let id = q.group();
        match out.iter_mut().find(|(g, _)| *g == id) {
            Some((_, qs)) => qs.push(q),
            None => out.push((id, vec![q])),
        }
    }
    Ok(out)
}

/// The answers a worker tries again by itself. A core row has no pane, so it
/// is in no listing until core delivery lands.
fn retried(store: &Store) -> rusqlite::Result<BTreeSet<i64>> {
    Ok(store.undelivered_all(&CAPS)?.iter().map(|q| q.id).collect())
}

fn marks(store: &Store) -> rusqlite::Result<(i64, i64)> {
    let highest = |table: &str| {
        store.db().query_row(
            &format!("SELECT COALESCE(MAX(id), 0) FROM {table}"),
            [],
            |r| r.get(0),
        )
    };
    Ok((highest("questions")?, highest("alerts")?))
}

fn omitted_none() -> Value {
    json!({"groups": 0, "questions": 0, "failed": 0, "alerts": 0, "remote": 0, "question_ids": []})
}

fn feed(
    wall: SystemTime,
    ok: bool,
    errors: Vec<String>,
    omitted: Value,
    (question_id, alert_id): (i64, i64),
    entries: Vec<Value>,
) -> Value {
    let at = wall.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    json!({
        "v": 1,
        "generated_at": at,
        "ok": ok,
        "errors": errors,
        "omitted": omitted,
        "new_since": {"question_id": question_id, "alert_id": alert_id},
        "entries": entries,
    })
}

#[cfg(test)]
mod tests;
