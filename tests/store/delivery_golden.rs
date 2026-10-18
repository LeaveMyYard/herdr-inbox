//! The delivery-state reads and writes replay tests/fixtures/store-delivery.json:
//! Python's `store.Store` over `STORE_DELIVERY_OPS` (claims, pending, busy
//! misses, failures, agents gone, panes moved, hook sends verified or not,
//! legacy checks, notifications, continuations, liveness), with every touched
//! table after each step. The Rust store runs the same script and must leave
//! the same rows and return the same results.

use std::collections::HashMap;
use std::time::Duration;

use herdr_inbox::store::questions::NewQuestion;
use herdr_inbox::store::{Claim, Store};
use serde_json::{Value, json};

use crate::common::migrated;
use crate::golden::{i, list, mask, question, s, sql, tables};

const GOLDEN: &str = include_str!("../fixtures/store-delivery.json");
const TABLES: [&str; 3] = ["questions", "question_ext", "sessions"];

fn secs(a: &Value, k: &str) -> Duration {
    Duration::from_secs(a[format!("{k}_s")].as_u64().unwrap())
}

fn b(a: &Value, k: &str) -> bool {
    a[k].as_bool().unwrap()
}

fn ids(a: &Value) -> Vec<i64> {
    serde_json::from_value(a["qids"].clone()).unwrap()
}

fn caps(a: &Value) -> Vec<(&str, i64)> {
    a["caps"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_i64().unwrap()))
        .collect()
}

fn opt<T: Into<Value>>(v: Option<T>) -> Value {
    v.map_or(Value::Null, Into::into)
}

struct Replay {
    store: Store,
    claims: HashMap<i64, Claim>,
}

impl Replay {
    /// The claim an argument names: `$claim:N` (the last claim on N) or a literal.
    fn claim(&self, a: &Value) -> Option<Claim> {
        let text = s(a, "claim")?;
        Some(match text.strip_prefix("$claim:") {
            Some(n) => self.claims[&n.parse::<i64>().unwrap()].clone(),
            None => Claim(text.into()),
        })
    }

    fn run(&mut self, op: &str, a: &Value) -> Value {
        let st = &self.store;
        let qid = || i(a, "qid");
        let claim = self.claim(a);
        match op {
            "sql" => {
                sql(st, a);
                Value::Null
            }
            "insert" => {
                let new = NewQuestion {
                    question: s(a, "question").unwrap(),
                    session_id: s(a, "session_id"),
                    pane_id: s(a, "pane_id"),
                    ..NewQuestion::default()
                };
                question(&st.insert(&new).unwrap())
            }
            "set_answer" => json!(st.set_answer(qid(), s(a, "answer").unwrap(), None).unwrap()),
            "claim_delivery" => {
                let got = st.claim_delivery(qid()).unwrap();
                if let Some(c) = &got {
                    self.claims.insert(qid(), c.clone());
                }
                got.map_or(Value::Null, |c| mask(&c.0))
            }
            "record_delivery" => {
                st.record_delivery(qid(), s(a, "error"), true, claim.as_ref(), false)
                    .unwrap();
                Value::Null
            }
            "record_hook_delivery" => {
                json!(st.record_hook_delivery(qid(), &claim.unwrap()).unwrap())
            }
            "acknowledge" => {
                st.acknowledge(&ids(a)).unwrap();
                Value::Null
            }
            "hide_failed" => {
                st.hide_failed(qid()).unwrap();
                Value::Null
            }
            "failed_deliveries" => list(st.failed_deliveries().unwrap()),
            "undelivered" => list(st.undelivered(s(a, "pane_id").unwrap(), &caps(a)).unwrap()),
            "undelivered_all" => list(st.undelivered_all(&caps(a)).unwrap()),
            "stuck_answers" => list(st.stuck_answers().unwrap()),
            "failed_pile" => list(st.failed_pile(secs(a, "within")).unwrap()),
            "clear_seen_errors" => json!(st.clear_seen_errors().unwrap()),
            "set_pending" => {
                st.set_pending(qid(), &claim.unwrap(), s(a, "reason").unwrap())
                    .unwrap();
                Value::Null
            }
            "record_busy_miss" => {
                st.record_busy_miss(qid(), &claim.unwrap(), s(a, "reason").unwrap())
                    .unwrap();
                Value::Null
            }
            "busy_attempts" => json!(st.busy_attempts(qid()).unwrap()),
            "get_pending_since" => st
                .pending_since(qid())
                .unwrap()
                .map_or(Value::Null, |t| mask(&t)),
            "hook_misses" => json!(st.hook_misses(qid()).unwrap()),
            "agent_gone" => json!(st.agent_gone(qid()).unwrap()),
            "legacy_check_due" => json!(st.legacy_check_due(qid(), secs(a, "every")).unwrap()),
            "legacy_miss" => json!(st.legacy_miss(qid()).unwrap()),
            "set_server" => {
                st.set_server(qid(), s(a, "server")).unwrap();
                Value::Null
            }
            "server_of" => opt(st.server_of(qid()).unwrap()),
            "notified" => json!(st.notified(qid()).unwrap()),
            "mark_notified" => {
                st.mark_notified(&ids(a)).unwrap();
                Value::Null
            }
            "mark_agent_gone" => {
                st.mark_agent_gone(qid(), s(a, "reason").unwrap()).unwrap();
                Value::Null
            }
            "move_to_pane" => json!(
                st.move_to_pane(
                    qid(),
                    s(a, "pane_id").unwrap(),
                    s(a, "tab_id"),
                    s(a, "reason").unwrap(),
                    claim.as_ref(),
                    s(a, "tab_label")
                )
                .unwrap()
            ),
            "gone_recent" => list(st.gone_recent(secs(a, "within")).unwrap()),
            "gone_candidates" => {
                let prefixes: Vec<&str> = a["prefixes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap())
                    .collect();
                list(st.gone_candidates(&prefixes, i(a, "cap")).unwrap())
            }
            "archive_gone" => json!(st.archive_gone(secs(a, "after")).unwrap()),
            "mark_seen_elsewhere" => json!(st.mark_seen_elsewhere(qid(), claim.as_ref()).unwrap()),
            "mark_landed" => json!(st.mark_landed(qid(), b(a, "acknowledge")).unwrap()),
            "hook_unverified" => Value::Array(
                st.hook_unverified(s(a, "server").unwrap())
                    .unwrap()
                    .iter()
                    .map(|(q, sent)| json!([question(q), mask(sent)]))
                    .collect(),
            ),
            "hook_verified" => {
                st.hook_verified(qid(), b(a, "acknowledge")).unwrap();
                Value::Null
            }
            "hook_unseen" => json!(
                st.hook_unseen(qid(), s(a, "sent_at").unwrap(), s(a, "reason"))
                    .unwrap()
            ),
            "continued_in" => opt(st.continued_in(s(a, "session_id")).unwrap()),
            "record_continuation" => {
                st.record_continuation(s(a, "old").unwrap(), s(a, "new").unwrap())
                    .unwrap();
                Value::Null
            }
            "watch_live" => json!(
                st.watch_live(s(a, "session_id"), secs(a, "within"))
                    .unwrap()
            ),
            "hook_live" => json!(st.hook_live(s(a, "session_id"), secs(a, "within")).unwrap()),
            other => panic!("no Rust call for {other}"),
        }
    }
}

#[test]
fn every_delivery_step_leaves_pythons_rows_and_returns_pythons_result() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
    let mut replay = Replay {
        store,
        claims: HashMap::new(),
    };
    let steps = golden["steps"].as_array().unwrap();
    assert!(steps.len() >= 120);
    for (n, step) in steps.iter().enumerate() {
        let op = step["op"].as_str().unwrap();
        let got = replay.run(op, &step["args"]);
        assert_eq!(got, step["result"], "step {n} {op}: result");
        assert_eq!(
            tables(&replay.store, &TABLES),
            step["tables"],
            "step {n} {op}: tables"
        );
    }
}
