//! The hook and rewake rows of the delivery decision table
//! (tests/fixtures/decisions.json, captured from the Python bridge by
//! tools/capture_goldens.py) reproduced by the policy. The worker's rows
//! (`deliver_batch`, `verify_*`) join in P5.
//!
//! A key missing from a row's `observed` was not looked up: the code decided
//! before it needed it (tests/fixtures/README.md).

use std::collections::BTreeSet;

use inbox_core::hook::{Candidate, Carry, carry, own_session};
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

/// The hook's gates: a row without a question stopped before any claim.
fn gates_pass(row: &Value) -> bool {
    let obs = &row["observed"];
    let own = own_session(flag(obs, "own_session").unwrap().then_some("s"), false);
    own && flag(obs, "has_candidates") == Some(true) && flag(obs, "schema_ready") != Some(false)
}

#[test]
fn every_hook_row_is_the_policys() {
    let rows = rows("hook");
    assert!(rows.len() >= 5);
    for row in &rows {
        let Some(question) = row.get("question") else {
            assert!(
                !gates_pass(row),
                "a gate row the policy would carry on from: {row}"
            );
            continue;
        };
        assert!(gates_pass(row), "{row}");
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
            // ended at the owner check; with the ownership intact only a gone Claude
            // process ends it there (its liveness is not a recorded lookup)
            Watch::owner(still_owner, !still_owner)
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
