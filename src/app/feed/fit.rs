//! What stays under 1 MiB (`app-feed-v1.md`, "Rows that still do not fit").
//! **SAFETY**: a row left out here is an open question nobody sees, so every
//! one is counted, and an open question's id is published.

use serde_json::{Value, json};

/// `readTextFile` reads no more than this.
pub const MAX_FEED: usize = 1024 * 1024;
/// Kept for the feed's own fields: 1000 ids of 16 digits, two error lines, the counts.
pub const HEAD_ROOM: usize = 24 * 1024;
/// The most ids `omitted.question_ids` holds.
pub const MAX_IDS: usize = 1000;

/// What kind of need a row has, in the order rows are fitted.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Need {
    /// A local group, with its open questions' ids.
    Group(Vec<i64>),
    /// A local alert.
    Alert,
    /// A `failed` row.
    Failed,
    /// A remote row of any kind.
    Remote,
}

/// One row, in display order.
#[derive(Debug)]
pub struct Item {
    /// The row.
    pub entry: Value,
    /// How it is counted.
    pub need: Need,
}

/// Walks `items` by need, skipping what does not fit in `room` bytes: one huge
/// row never takes the rows behind it with it. The rows that fit, still in
/// display order, and the `omitted` object.
pub fn fit(items: Vec<Item>, room: usize) -> (Vec<Value>, Value) {
    let sized: Vec<(usize, Item)> = items
        .into_iter()
        .map(|item| (serialized(&item.entry) + 1, item))
        .collect();
    let mut order: Vec<usize> = (0..sized.len()).collect();
    order.sort_by_key(|&i| (rank(&sized[i].1.need), first_id(&sized[i].1.need), i));

    let mut left = room;
    let mut keep = vec![false; sized.len()];
    let (mut groups, mut questions, mut failed, mut alerts, mut remote) = (0, 0, 0, 0, 0);
    let mut ids: Vec<i64> = Vec::new();
    for i in order {
        let (size, item) = &sized[i];
        if *size <= left {
            left -= size;
            keep[i] = true;
            continue;
        }
        match &item.need {
            Need::Group(open) => {
                groups += 1;
                questions += open.len();
                ids.extend(open);
            }
            Need::Alert => alerts += 1,
            Need::Failed => failed += 1,
            Need::Remote => remote += 1,
        }
    }
    ids.sort_unstable();
    ids.truncate(MAX_IDS);
    let kept = sized
        .into_iter()
        .zip(keep)
        .filter(|(_, keep)| *keep)
        .map(|((_, item), _)| item.entry)
        .collect();
    let omitted = json!({
        "groups": groups, "questions": questions, "failed": failed,
        "alerts": alerts, "remote": remote, "question_ids": ids,
    });
    (kept, omitted)
}

fn rank(need: &Need) -> u8 {
    match need {
        Need::Group(_) => 0,
        Need::Alert => 1,
        Need::Failed => 2,
        Need::Remote => 3,
    }
}

/// Oldest first among groups: the walk is by need, not by display order.
fn first_id(need: &Need) -> i64 {
    match need {
        Need::Group(ids) => ids.first().copied().unwrap_or(0),
        _ => 0,
    }
}

/// Walks `rows` in order, keeping each that fits in what is left of `room`
/// bytes: rows that inform and are no open question, fitted after every
/// entry, so they never crowd one out. The rows kept, still in order, and how
/// many were left out.
pub fn fit_rest(rows: Vec<Value>, room: usize) -> (Vec<Value>, usize) {
    let mut left = room;
    let mut omitted = 0;
    let mut kept = Vec::new();
    for row in rows {
        let size = serialized(&row) + 1;
        if size <= left {
            left -= size;
            kept.push(row);
        } else {
            omitted += 1;
        }
    }
    (kept, omitted)
}

/// What `rows` take in the feed: each serialized, and its comma.
pub fn size_of(rows: &[Value]) -> usize {
    rows.iter()
        .map(|r| serialized(r).saturating_add(1))
        .fold(0, usize::saturating_add)
}

fn serialized(v: &Value) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

/// How many rows `omitted` counts, for its `errors` line.
pub fn left_out(omitted: &Value) -> u64 {
    ["groups", "failed", "alerts", "remote"]
        .iter()
        .filter_map(|k| omitted[*k].as_u64())
        .sum()
}

#[cfg(test)]
mod tests;
