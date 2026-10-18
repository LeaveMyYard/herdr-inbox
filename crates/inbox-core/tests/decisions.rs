//! The hook and rewake rows of the delivery decision table
//! (tests/fixtures/decisions.json, captured from the Python bridge by
//! tools/capture_goldens.py) reproduced by the policy. The worker's rows
//! (`deliver_batch`, `verify_*`) join in P5.
//!
//! A key missing from a row's `observed` was not looked up: the code decided
//! before it needed it (tests/fixtures/README.md).

use std::collections::BTreeSet;

use inbox_core::hook::{Candidate, Carry, Gate, carry, gate};
use inbox_core::rewake::{Ready, UNKNOWN_SESSION_END, Verdict, Watch};
use serde_json::Value;

const TABLE: &str = include_str!("../../../tests/fixtures/decisions.json");

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

fn flag(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

/// The hook's gates replayed on a row's recorded lookups: a lookup the row
/// does not hold was not made, so asking for it fails the test. The schema
/// check is recorded only when it fails (Python notes `schema_ready` on
/// `SchemaNotReady`), so its absence means ready.
fn gate_of(row: &Value) -> Gate {
    let obs = &row["observed"];
    let recorded =
        |key: &str| flag(obs, key).unwrap_or_else(|| panic!("{key} was not looked up: {row}"));
    gate(
        recorded("own_session"),
        || recorded("has_candidates"),
        || flag(obs, "schema_ready").unwrap_or(true),
    )
}

#[test]
fn every_hook_row_is_the_policys() {
    let rows = rows("hook");
    assert!(rows.len() >= 5);
    for row in &rows {
        let Some(question) = row.get("question") else {
            assert_ne!(
                gate_of(row),
                Gate::Carry,
                "a gate row the policy would carry on from: {row}"
            );
            continue;
        };
        assert_eq!(gate_of(row), Gate::Carry, "{row}");
        let landed: Option<BTreeSet<usize>> =
            serde_json::from_value(question["landed"].clone()).unwrap();
        let c = Candidate {
            claimed: flag(question, "claimed").unwrap(),
            landed: landed.as_ref(),
            in_this_message: flag(question, "in_this_message").unwrap_or(false),
        };
        let (before, after) = (&row["before"], &row["after"]);
        match carry(&c) {
            Carry::Send => {
                assert_eq!(flag(question, "recorded"), Some(true), "{row}");
                assert_eq!(
                    (after["state"].as_str(), flag(after, "hook_sent")),
                    (Some("delivered"), Some(true)),
                    "{row}"
                );
                assert_eq!(
                    after.get("pending_since"),
                    None,
                    "the busy grace ends: {row}"
                );
            }
            Carry::Landed { acknowledge } => {
                assert_eq!(after["state"], "delivered", "{row}");
                assert_eq!(flag(after, "verified"), Some(true), "{row}");
                assert_eq!(
                    flag(after, "acknowledged").unwrap_or(false),
                    acknowledge,
                    "{row}"
                );
            }
            Carry::NextFiring | Carry::NotClaimed => assert_eq!(before, after, "{row}"),
        }
    }
}

#[test]
fn every_rewake_row_is_the_policys() {
    let rows = rows("rewake");
    assert!(rows.len() >= 9);
    for row in &rows {
        let obs = &row["observed"];
        let want = row["action"].as_str().unwrap();
        let got = if let Some(still_owner) = flag(obs, "still_owner") {
            // ended at the poll's first check; with the ownership intact and the watch
            // young, only a gone agent process ends it there (its liveness is not a
            // recorded lookup)
            Watch::new(0.0).begin_poll(1.0, still_owner, !still_owner)
        } else {
            let ready = match obs["ready"].as_str().unwrap() {
                "go" => Ready::Go,
                "wait" => Ready::Wait,
                "unknown" => Ready::Unknown,
                "end" => Ready::End,
                other => panic!("{other}"),
            };
            let pending = flag(obs, "pending").unwrap();
            let mut watch = Watch::new(0.0);
            let now = if flag(obs, "unknown_too_long") == Some(true) {
                watch.after_look(pending, Ready::Unknown, 0.0);
                UNKNOWN_SESSION_END
            } else {
                0.0
            };
            match watch.after_look(pending, ready, now) {
                Verdict::Deliver => Watch::after_delivery(flag(obs, "hook_sent").unwrap()),
                other => {
                    assert_eq!(obs.get("hook_sent"), None, "no delivery was tried: {row}");
                    other
                }
            }
        };
        assert_eq!(got.name(), want, "{row}");
    }
}
