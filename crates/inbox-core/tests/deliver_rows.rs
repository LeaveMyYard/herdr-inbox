//! The `deliver_batch` rows of the delivery decision table
//! (tests/fixtures/decisions.json, captured from the Python worker by
//! tools/capture_goldens.py) reproduced by `inbox_core::deliver::decide`.
//!
//! Each row is one answer of one pane's pass: its inputs, every lookup Python
//! made (a key missing from `observed`/`question` was not looked up: asking
//! for it fails the test), and the answer's state before and after. The
//! decision's outcome, applied to `before` as the store write would, must give
//! `after`.

use inbox_core::deliver::{
    Agent, BUSY_WAIT_IDLE, Composer, Landing, Look, Mode, Outcome, REQUEUED_WAIT, Waiting, decide,
    older_error,
};
use serde_json::{Map, Value, json};

const TABLE: &str = include_str!("../../../tests/fixtures/decisions.json");
const PANE: &str = "w1:p1";
const ASKER: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const LIVE: &str = "bbbbbbbb-0000-4000-8000-00000000000b";
const REPORTED: &str = "cccccccc-0000-4000-8000-00000000000c";

fn rows() -> Vec<Value> {
    let table: Value = serde_json::from_str(TABLE).unwrap();
    table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["path"] == "deliver_batch")
        .cloned()
        .collect()
}

/// The recorder's masking of free text (tests/decision_recorder.py `text`) for
/// the ids this replay uses.
fn mask(s: &str) -> String {
    let mut out = s.replace(PANE, "<pane>");
    for id in [ASKER, LIVE, REPORTED] {
        out = out.replace(id, "<session>").replace(&id[..8], "<session>");
    }
    out
}

fn agent_of(v: &Value) -> Option<Agent> {
    v.as_object().map(|a| Agent {
        kind: a.get("kind").and_then(Value::as_str).map(String::from),
        status: a["status"].as_str().unwrap().into(),
        focused: a["focused"].as_bool().unwrap(),
        session: a["session_known"]
            .as_bool()
            .unwrap()
            .then(|| LIVE.to_string()),
        reported_session: Some(REPORTED.into()),
    })
}

/// A row's recorded lookups: a lookup it does not hold was not made.
struct RowLook<'a> {
    row: &'a Value,
}

impl RowLook<'_> {
    fn pane(&self, key: &str) -> &Value {
        self.row["observed"]
            .get(key)
            .unwrap_or_else(|| panic!("{key} was not looked up: {}", self.row))
    }
    fn of(&self, key: &str) -> &Value {
        self.row["question"]
            .get(key)
            .unwrap_or_else(|| panic!("{key} was not looked up: {}", self.row))
    }
    fn flag(&self, key: &str) -> bool {
        self.of(key).as_bool().unwrap()
    }
}

impl Look for RowLook<'_> {
    fn claimed(&mut self, _: i64) -> bool {
        self.flag("claimed")
    }
    fn agent(&mut self) -> Option<Agent> {
        agent_of(self.pane("agent"))
    }
    fn moved_elsewhere(&mut self, _: i64) -> bool {
        self.flag("moved_elsewhere")
    }
    fn conversation_changed(&mut self, _: i64) -> bool {
        self.flag("conversation_changed")
    }
    fn pane_runs_older_session(&mut self, _: i64) -> bool {
        self.flag("pane_runs_older_session")
    }
    fn watched(&mut self) -> bool {
        self.pane("watched").as_bool().unwrap()
    }
    fn hook_misses(&mut self, _: i64) -> i64 {
        self.of("hook_misses").as_i64().unwrap()
    }
    fn hook_live(&mut self) -> bool {
        self.pane("hook_live").as_bool().unwrap()
    }
    fn busy_attempts(&mut self, _: i64) -> i64 {
        self.of("busy_attempts").as_i64().unwrap()
    }
    fn within_busy_grace(&mut self, _: i64) -> bool {
        self.flag("within_busy_grace")
    }
    fn panel_focused(&mut self) -> bool {
        self.pane("panel_focused").as_bool().unwrap()
    }
    fn landed(&mut self, ids: &[i64]) -> Vec<Option<Vec<usize>>> {
        self.pane("transcript_known"); // noted with the landings
        let landed: Option<Vec<usize>> = serde_json::from_value(self.of("landed").clone()).unwrap();
        ids.iter().map(|_| landed.clone()).collect()
    }
    fn pane_reports_another_session(&mut self) -> bool {
        self.pane("pane_reports_another_session").as_bool().unwrap()
    }
    fn already_typed_into_older_session(&mut self, _: i64) -> bool {
        self.flag("already_typed_into_older_session")
    }
    fn messages(&mut self, ids: &[i64]) -> Vec<(Vec<i64>, bool)> {
        vec![(ids.to_vec(), self.flag("full_answer_in_message"))]
    }
    fn agent_again(&mut self, _: &[i64]) -> Option<Agent> {
        // the recorder keeps no session ids: the same session is assumed when one is known
        agent_of(self.of("agent_again"))
    }
    fn composer(&mut self, _: &[i64]) -> Composer {
        match self.of("composer").as_str().unwrap() {
            "empty" => Composer::Empty,
            "draft" => Composer::Draft,
            _ => Composer::Unknown,
        }
    }
    fn prompt(&mut self, _: &[i64]) -> Option<String> {
        self.of("prompt_error").as_str().map(String::from)
    }
    fn verify_transcript(&mut self, _: &[i64]) -> bool {
        self.flag("verify_transcript")
    }
    fn landing(&mut self, _: i64) -> Landing {
        Landing {
            counted: self.flag("landed_after_typing"),
            anywhere: self.flag("landed_anywhere_after_typing"),
            older_error: self
                .flag("older_session_watched")
                .then(|| older_error(PANE, Some(REPORTED), Some(ASKER), true)),
        }
    }
}

/// `before` as the store write of `outcome` leaves it (the recorder's state view).
fn applied(before: &Value, outcome: &Outcome) -> Value {
    let mut s: Map<String, Value> = before.as_object().unwrap().clone();
    let attempts = s.get("attempts").and_then(Value::as_i64).unwrap_or(0);
    let mut put = |k: &str, v: Option<Value>| match v {
        Some(v) => {
            s.insert(k.into(), v);
        }
        None => {
            s.remove(k);
        }
    };
    match outcome {
        Outcome::Untouched => {}
        Outcome::Failed(error) => {
            put("state", Some(json!("failed")));
            put("reason", Some(json!(mask(error))));
            put("claim_stale", None);
            put("attempts", Some(json!(attempts + 1)));
        }
        Outcome::Delivered { ack } => {
            put("state", Some(json!("delivered")));
            put("reason", None);
            put("claim_stale", None);
            put("pending_since", None);
            if *ack {
                put("acknowledged", Some(json!(true)));
            }
            put("verified", Some(json!(true)));
        }
        Outcome::SentUnverified => {
            put("state", Some(json!("delivered")));
            put("reason", None);
            put("claim_stale", None);
            put("pending_since", None);
        }
        Outcome::Pending(reason) => {
            put("state", Some(json!("pending")));
            put("reason", Some(json!(mask(reason))));
            put("claim_stale", None);
            if reason.starts_with("busy:") {
                put("pending_since", Some(json!(true)));
            }
        }
        Outcome::BusyMiss => {
            let busy = before
                .get("busy_attempts")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            put("state", Some(json!("pending")));
            put("reason", Some(json!(BUSY_WAIT_IDLE)));
            put("claim_stale", None);
            put("busy_attempts", Some(json!(busy + 1)));
        }
        Outcome::Moved => {
            put("state", Some(json!("pending")));
            put(
                "reason",
                Some(json!("moved: the asking agent is now in pane <pane>")),
            );
            put("claim_stale", None);
            put("attempts", None);
            put("agent_gone_marked", None);
            put("moved_to_another_pane", Some(json!(true)));
        }
        Outcome::VerifyLater => {
            put("state", Some(json!("claimed")));
            put("reason", None);
            put("claim_stale", Some(json!(false)));
        }
    }
    Value::Object(s)
}

#[test]
fn every_deliver_batch_row_is_the_policys() {
    let rows = rows();
    assert!(rows.len() >= 134);
    for row in &rows {
        let inputs = &row["inputs"];
        let mode = Mode {
            retry: inputs["retry"].as_bool().unwrap(),
            force: inputs["force"].as_bool().unwrap(),
            verify_later: inputs["verify_later"].as_bool().unwrap(),
        };
        // Python's `bool(pane)`: no pane and an empty one are both recorded as `pane: false`
        let panes: &[Option<&str>] = match row["observed"].get("pane").and_then(Value::as_bool) {
            Some(false) => &[None, Some("")],
            _ => &[Some(PANE)],
        };
        let requeued = row["before"]["reason"].as_str() == Some(REQUEUED_WAIT);
        let waiting = [Waiting {
            id: 1,
            session: Some(ASKER.into()),
            requeued,
        }];
        for &pane in panes {
            let mut look = RowLook { row };
            let [(_, outcome)]: [(i64, Outcome); 1] =
                decide(pane, &waiting, mode, &mut look).try_into().unwrap();
            assert_eq!(
                applied(&row["before"], &outcome),
                row["after"],
                "{outcome:?}\n{row}"
            );
        }
    }
}
