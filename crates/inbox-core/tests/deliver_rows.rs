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
        // the recorder's per-frame label (s1 is the pane's first agent's session)
        session: a["session"].as_str().map(|label| match label {
            "s1" => LIVE.to_string(),
            other => format!("{other}-other-session"),
        }),
        reported_session: Some(REPORTED.into()),
    })
}

/// One pass's recorded lookups (its rows, answer `slot + 1` each): a lookup
/// a row does not hold was not made.
struct PassLook<'a> {
    rows: &'a [&'a Value],
}

impl PassLook<'_> {
    fn row(&self, id: i64) -> &Value {
        self.rows[(id - 1) as usize]
    }
    fn pane(&self, key: &str) -> &Value {
        let row = self.rows[0];
        row["observed"]
            .get(key)
            .unwrap_or_else(|| panic!("{key} was not looked up: {row}"))
    }
    fn of(&self, id: i64, key: &str) -> &Value {
        let row = self.row(id);
        row["question"]
            .get(key)
            .unwrap_or_else(|| panic!("{key} was not looked up for {id}: {row}"))
    }
    fn flag(&self, id: i64, key: &str) -> bool {
        self.of(id, key).as_bool().unwrap()
    }
}

impl Look for PassLook<'_> {
    fn claimed(&mut self, id: i64) -> bool {
        self.flag(id, "claimed")
    }
    fn agent(&mut self) -> Option<Agent> {
        agent_of(self.pane("agent"))
    }
    fn moved_elsewhere(&mut self, id: i64) -> bool {
        self.flag(id, "moved_elsewhere")
    }
    fn conversation_changed(&mut self, id: i64) -> bool {
        self.flag(id, "conversation_changed")
    }
    fn pane_runs_older_session(&mut self, id: i64) -> bool {
        self.flag(id, "pane_runs_older_session")
    }
    fn watched(&mut self) -> bool {
        self.pane("watched").as_bool().unwrap()
    }
    fn hook_misses(&mut self, id: i64) -> i64 {
        self.of(id, "hook_misses").as_i64().unwrap()
    }
    fn hook_live(&mut self) -> bool {
        self.pane("hook_live").as_bool().unwrap()
    }
    fn busy_attempts(&mut self, id: i64) -> i64 {
        self.of(id, "busy_attempts").as_i64().unwrap()
    }
    fn within_busy_grace(&mut self, id: i64) -> bool {
        self.flag(id, "within_busy_grace")
    }
    fn panel_focused(&mut self) -> bool {
        self.pane("panel_focused").as_bool().unwrap()
    }
    fn landed(&mut self, ids: &[i64]) -> Vec<Option<Vec<usize>>> {
        self.pane("transcript_known"); // noted with the landings
        ids.iter()
            .map(|id| serde_json::from_value(self.of(*id, "landed").clone()).unwrap())
            .collect()
    }
    fn pane_reports_another_session(&mut self) -> bool {
        self.pane("pane_reports_another_session").as_bool().unwrap()
    }
    fn already_typed_into_older_session(&mut self, id: i64) -> bool {
        self.flag(id, "already_typed_into_older_session")
    }
    fn messages(&mut self, ids: &[i64]) -> Vec<(Vec<i64>, bool)> {
        // the recorded batches, in typing order, each with its full-answer flag
        let mut out: Vec<(i64, Vec<i64>, bool)> = Vec::new();
        for &id in ids {
            let n = self.of(id, "message").as_i64().unwrap();
            let full = self.flag(id, "full_answer_in_message");
            match out.iter_mut().find(|(m, _, _)| *m == n) {
                Some((_, batch, _)) => batch.push(id),
                None => out.push((n, vec![id], full)),
            }
        }
        out.sort_by_key(|(n, _, _)| *n);
        out.into_iter()
            .map(|(_, batch, full)| (batch, full))
            .collect()
    }
    fn agent_again(&mut self, ids: &[i64]) -> Option<Agent> {
        agent_of(self.of(ids[0], "agent_again"))
    }
    fn composer(&mut self, ids: &[i64]) -> Composer {
        match self.of(ids[0], "composer").as_str().unwrap() {
            "empty" => Composer::Empty,
            "draft" => Composer::Draft,
            _ => Composer::Unknown,
        }
    }
    fn prompt(&mut self, ids: &[i64]) -> Option<String> {
        self.of(ids[0], "prompt_error").as_str().map(String::from)
    }
    fn verify_transcript(&mut self, ids: &[i64]) -> bool {
        self.flag(ids[0], "verify_transcript")
    }
    fn landing(&mut self, id: i64) -> Landing {
        Landing {
            counted: self.flag(id, "landed_after_typing"),
            anywhere: self.flag(id, "landed_anywhere_after_typing"),
            older_error: self
                .flag(id, "older_session_watched")
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
fn every_deliver_batch_pass_is_the_policys() {
    let rows = rows();
    assert!(rows.len() >= 148);
    // a pass: the rows of one pane's deliver_batch, its answers in their order
    let mut passes: Vec<(&str, Vec<&Value>)> = Vec::new();
    for row in &rows {
        let id = row["pass"].as_str().unwrap();
        match passes.iter_mut().find(|(p, _)| *p == id) {
            Some((_, list)) => list.push(row),
            None => passes.push((id, vec![row])),
        }
    }
    let mut several = 0;
    for (_, list) in &mut passes {
        list.sort_by_key(|r| r["slot"].as_i64().unwrap());
        assert!(
            list.iter().enumerate().all(|(i, r)| r["slot"] == i as i64),
            "a whole pass"
        );
        several += usize::from(list.len() > 1);
        let first = list[0];
        let inputs = &first["inputs"];
        let mode = Mode {
            retry: inputs["retry"].as_bool().unwrap(),
            force: inputs["force"].as_bool().unwrap(),
            verify_later: inputs["verify_later"].as_bool().unwrap(),
        };
        // Python's `bool(pane)`: no pane and an empty one are both recorded as `pane: false`
        let panes: &[Option<&str>] = match first["observed"].get("pane").and_then(Value::as_bool) {
            Some(false) => &[None, Some("")],
            _ => &[Some(PANE)],
        };
        let waiting: Vec<Waiting> = list
            .iter()
            .zip(1..)
            .map(|(r, id)| Waiting {
                id,
                session: Some(ASKER.into()),
                requeued: r["before"]["reason"].as_str() == Some(REQUEUED_WAIT),
            })
            .collect();
        for &pane in panes {
            let mut look = PassLook { rows: list };
            let got = decide(pane, &waiting, mode, &mut look);
            assert_eq!(got.len(), list.len());
            for ((id, outcome), row) in got.iter().zip(list.iter()) {
                assert_eq!(
                    applied(&row["before"], outcome),
                    row["after"],
                    "answer {id}: {outcome:?}\n{row}"
                );
            }
        }
    }
    assert!(several >= 10, "passes with several answers: {several}");
}
