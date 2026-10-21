//! The `verify_hook` and `verify_job` rows of the delivery decision table
//! (tests/fixtures/decisions.json, captured from the Python worker by
//! tools/capture_goldens.py) reproduced by `inbox_core::verify`.
//!
//! Each row is one answer: its inputs, the lookups Python made (a key missing
//! from `observed`/`question` was not looked up: asking for it fails the
//! test), and its state before and after. The outcome, applied to `before`
//! as the store write would, must give `after`.

use inbox_core::deliver::{BUSY_WAIT_IDLE, Outcome};
use inbox_core::verify::{HOOK_UNSEEN, HookLook, HookOutcome, JobLook, Landing, hook, job};
use serde_json::{Map, Value, json};

const TABLE: &str = include_str!("../../../tests/fixtures/decisions.json");
/// An older-session state as the recorder masks it (the row keeps only that one was watched).
const OLDER: &str = "older conversation: typed into <session> in pane <pane>, an older session than the asking \
                     <session>, which never got it; resume <session> to deliver";

fn rows(path: &str) -> Vec<Value> {
    let table: Value = serde_json::from_str(TABLE).unwrap();
    table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["path"] == path)
        .cloned()
        .collect()
}

/// A recorded lookup: one the row does not hold was not made.
fn looked<'a>(row: &'a Value, field: &str, key: &str) -> &'a Value {
    row[field]
        .get(key)
        .unwrap_or_else(|| panic!("{key} was not looked up: {row}"))
}

struct RowHook<'a>(&'a Value);

impl HookLook for RowHook<'_> {
    fn landed(&mut self) -> Option<Vec<usize>> {
        serde_json::from_value(looked(self.0, "question", "landed").clone()).unwrap()
    }
    fn verify_time_up(&mut self) -> bool {
        looked(self.0, "question", "verify_time_up")
            .as_bool()
            .unwrap()
    }
    fn transcript_known(&mut self) -> bool {
        looked(self.0, "observed", "transcript_known")
            .as_bool()
            .unwrap()
    }
}

fn state(before: &Value) -> Map<String, Value> {
    before.as_object().unwrap().clone()
}

fn count(s: &Map<String, Value>, key: &str) -> i64 {
    s.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// `before` as the hook verification's store write leaves it.
fn hook_applied(before: &Value, outcome: HookOutcome) -> Value {
    let mut s = state(before);
    match outcome {
        HookOutcome::Wait => {}
        HookOutcome::Verified { ack } => {
            // hook_verified: verified, seen unless only the reference landed
            if ack {
                s.insert("acknowledged".into(), json!(true));
            }
            s.insert("verified".into(), json!(true));
        }
        HookOutcome::Unseen => {
            // hook_unseen(HOOK_UNSEEN): the send cleared, a miss counted, pending again
            let misses = count(&s, "hook_misses") + 1;
            s.remove("hook_sent");
            s.insert("hook_misses".into(), json!(misses));
            s.insert("state".into(), json!("pending"));
            s.insert("reason".into(), json!(HOOK_UNSEEN));
        }
        HookOutcome::SentUnverified => {
            // hook_unseen(None): the send cleared, nothing else
            s.remove("hook_sent");
        }
    }
    Value::Object(s)
}

/// `before` (a claimed row) as `_record_outcome`'s store write leaves it.
fn job_applied(before: &Value, outcome: &Outcome) -> Value {
    let mut s = state(before);
    s.remove("claim_stale");
    match outcome {
        Outcome::Delivered { ack } => {
            s.insert("state".into(), json!("delivered"));
            if *ack {
                s.insert("acknowledged".into(), json!(true));
            }
            s.insert("verified".into(), json!(true));
        }
        Outcome::Failed(error) => {
            let attempts = count(&s, "attempts") + 1;
            s.insert("state".into(), json!("failed"));
            s.insert("reason".into(), json!(error));
            s.insert("attempts".into(), json!(attempts));
        }
        Outcome::BusyMiss => {
            let busy = count(&s, "busy_attempts") + 1;
            s.insert("state".into(), json!("pending"));
            s.insert("reason".into(), json!(BUSY_WAIT_IDLE));
            s.insert("busy_attempts".into(), json!(busy));
        }
        other => panic!("not a landing's outcome: {other:?}"),
    }
    Value::Object(s)
}

#[test]
fn every_verify_hook_row_is_the_policys() {
    let rows = rows("verify_hook");
    assert!(rows.len() >= 6);
    for row in &rows {
        let outcome = hook(&mut RowHook(row));
        assert_eq!(
            hook_applied(&row["before"], outcome),
            row["after"],
            "{outcome:?}\n{row}"
        );
    }
}

/// One job's rows (answer `slot + 1` each): where each one's text landed.
struct RowJob<'a>(&'a [&'a Value]);

impl JobLook for RowJob<'_> {
    fn landing(&mut self, id: i64) -> Landing {
        let row = self.0[(id - 1) as usize];
        let flag = |key: &str| looked(row, "question", key).as_bool().unwrap();
        Landing {
            counted: flag("landed_after_typing"),
            anywhere: flag("landed_anywhere_after_typing"),
            older_error: flag("older_session_watched").then(|| OLDER.to_string()),
        }
    }
}

#[test]
fn every_verify_job_is_the_policys() {
    let rows = rows("verify_job");
    assert!(rows.len() >= 10);
    // a job: the rows of one `inbox verify` run, in their order
    let mut jobs: Vec<(&str, Vec<&Value>)> = Vec::new();
    for row in &rows {
        let id = row["pass"].as_str().unwrap();
        match jobs.iter_mut().find(|(p, _)| *p == id) {
            Some((_, list)) => list.push(row),
            None => jobs.push((id, vec![row])),
        }
    }
    let mut several = 0;
    for (_, list) in &mut jobs {
        list.sort_by_key(|r| r["slot"].as_i64().unwrap());
        assert!(
            list.iter().enumerate().all(|(i, r)| r["slot"] == i as i64),
            "a whole job"
        );
        several += usize::from(list.len() > 1);
        let input = |key: &str| list[0]["inputs"][key].as_bool().unwrap();
        let ids: Vec<i64> = (1..=list.len() as i64).collect();
        let got = job(&ids, input("ack"), input("busy"), &mut RowJob(list));
        assert_eq!(got.len(), list.len(), "every row of the job decided");
        for ((id, outcome), row) in got.iter().zip(list.iter()) {
            assert_eq!(
                job_applied(&row["before"], outcome),
                row["after"],
                "row {id}: {outcome:?}\n{row}"
            );
        }
    }
    assert!(several >= 2, "jobs with several rows: {several}");
}
