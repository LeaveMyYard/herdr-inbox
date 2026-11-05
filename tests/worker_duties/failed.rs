//! The failed pile reconciled (Python tests/test_failed_pile.py, the
//! worker's half): only the asker's FORWARD family is evidence, and only
//! its own inbox calls. Synthetic data, in the shapes the Python tests have.

use std::time::Duration;

use herdr_inbox::clock::{now, now_minus};
use herdr_inbox::store::question::normalize_options;
use herdr_inbox::store::questions::NewQuestion;
use inbox_core::deliver::NOT_VERIFIED;
use rusqlite::params;
use serde_json::{Value, json};

use crate::World;

pub const OLD: &str = "00000000-0000-4000-8000-000000000001";
pub const NEW: &str = "00000000-0000-4000-8000-000000000002";
const GONE_P48: &str = "agent gone: no live agent in pane w1:p4";
const CHANGED_P57: &str = "agent gone: conversation changed in pane w1:p5";
const CHECK_ANSWERS: &str = "mcp__plugin_herdr-inbox_herdr-inbox__check_answers";

impl World {
    /// Python `filed`: asked by `session` in `pane`, answered without a delivery.
    pub fn filed(&mut self, pane: &str, session: &str, text: &str, answer: &str) -> i64 {
        let q = self
            .store
            .insert(&NewQuestion {
                pane_id: Some(pane),
                tab_id: Some("w1:t3"),
                tab_label: Some("node"),
                session_id: Some(session),
                cwd: Some("/r"),
                question: text,
                options: normalize_options(&json!(null)).unwrap(),
                ..NewQuestion::default()
            })
            .unwrap();
        let server = self.server.clone();
        self.store.set_server(q.id, Some(&server)).unwrap();
        self.store.set_answer(q.id, answer, None).unwrap();
        q.id
    }

    /// Python `as_stored`: the row's delivery state as the database had it.
    pub fn as_stored(&self, qid: i64, error: &str, acknowledged: bool) {
        let ack = acknowledged.then(now);
        self.sql(
            "UPDATE questions SET delivery_error = ?, delivery_attempts = 4, acknowledged_at = ? WHERE id = ?",
            params![error, ack, qid],
        );
        if error.starts_with("agent gone") {
            self.sql(
                "INSERT INTO question_ext (id, group_id, agent_gone_at) VALUES (?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET agent_gone_at = excluded.agent_gone_at",
                params![qid, qid, now()],
            );
        }
    }

    /// The ids of the popup's failed pile.
    pub fn failed(&self) -> Vec<i64> {
        self.store
            .failed_deliveries()
            .unwrap()
            .iter()
            .map(|q| q.id)
            .collect()
    }

    /// Python `continued_then_resumed`: OLD continued in NEW, then herdr
    /// resumed OLD, whose first record after that is `batch` (if any).
    pub fn continued_then_resumed(&self, batch: Option<&str>) {
        self.write(OLD, &[
            json!({"type": "user", "message": {"role": "user", "content": "earlier"}}),
            json!({"type": "continued-in", "timestamp": "2026-01-02T03:04:05.678Z", "sessionId": OLD,
                   "continuedInSessionId": NEW}),
            json!({"type": "cost-state", "sessionId": OLD}),
        ]);
        self.write(
            NEW,
            &[json!({"type": "ai-title", "aiTitle": "x", "sessionId": NEW})],
        );
        if let Some(text) = batch {
            self.write(OLD, &[user(text)]);
        }
    }
}

pub fn user(text: &str) -> Value {
    json!({"type": "user", "timestamp": now(), "message": {"role": "user", "content": text}})
}

fn tool_use(name: &str, input: Value, id: &str) -> Value {
    json!({"type": "assistant", "timestamp": now(), "message": {"role": "assistant", "content": [
        {"type": "tool_use", "id": id, "name": name, "input": input}]}})
}

fn tool_result(id: &str, content: &str) -> Value {
    json!({"type": "user", "timestamp": now(), "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": id, "content": content}]}})
}

fn agents(pairs: &[(&str, &str)]) -> Value {
    pairs
        .iter()
        .map(|(p, s)| {
            json!({"pane_id": p, "tab_id": "w1:t4", "agent": "claude", "agent_status": "idle",
                             "agent_session": {"value": s}})
        })
        .collect()
}

fn untouched(w: &World, qid: i64) {
    let r = w.store.get(qid).unwrap().unwrap();
    assert!(r.acknowledged_at.is_none() && r.verified_at.is_none() && r.delivered_at.is_none());
}

#[test]
fn reconcile_never_verifies_answers_that_landed_only_in_an_older_session() {
    let mut w = World::new();
    let q1 = w.filed("w1:p3", NEW, "Question Text 101", "yes");
    let q2 = w.filed("w1:p3", NEW, "Question Text 102", "Answer Text 102");
    for q in [q1, q2] {
        w.as_stored(q, NOT_VERIFIED, false);
    }
    let batch = w.batch_text(&[q1, q2]);
    w.continued_then_resumed(Some(&batch));
    w.record_session(OLD);
    assert_eq!(w.failed(), [q1, q2]);
    w.full_tick();
    assert_eq!(w.failed(), [q1, q2]);
    untouched(&w, q1);
    untouched(&w, q2);
}

#[test]
fn reconcile_never_counts_a_changed_conversation_answer_found_in_an_older_session() {
    let mut w = World::new();
    let q = w.filed("w1:p5", NEW, "Question Text 103", "Answer Text 103");
    w.as_stored(q, CHANGED_P57, false);
    w.write(
        OLD,
        &[json!({"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW})],
    );
    w.record_session(OLD);
    let text = w.batch_text(&[q]);
    w.write(OLD, &[user(&text)]);
    w.fake.set(|k| k.agents = agents(&[("w1:p5", OLD)]));
    w.full_tick();
    untouched(&w, q);
    assert_eq!(w.failed(), [q]);
}

#[test]
fn an_acknowledged_answer_never_keeps_a_failure() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Question Text 106", "done");
    w.as_stored(q, GONE_P48, true);
    w.full_tick();
    assert_eq!(w.store.get(q).unwrap().unwrap().delivery_error, None);
}

#[test]
fn a_truly_gone_answer_that_never_landed_stays_visible() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Question Text 104", "Answer Text 104");
    w.as_stored(q, GONE_P48, false);
    w.fake.set(|k| k.agents = agents(&[("w1:p5", OLD)]));
    w.full_tick();
    assert_eq!(w.failed(), [q]);
}

#[test]
fn reconcile_backs_off_per_answer() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Backoff?", "x");
    w.as_stored(q, GONE_P48, false);
    w.full_tick();
    let first = w.memory.reconciled_at[&q];
    w.full_tick();
    assert_eq!(w.memory.reconciled_at[&q], first); // the second pass backed off
}

#[test]
fn q167_gone_answers_leave_the_popup_after_a_day() {
    let mut w = World::new();
    let old = w.filed("w1:p4", NEW, "Gone question 1?", "a");
    let new = w.filed("w1:p4", NEW, "Gone question 2?", "a");
    w.as_stored(old, GONE_P48, false);
    w.as_stored(new, GONE_P48, false);
    let day_ago = herdr_inbox::clock::now_minus(std::time::Duration::from_secs(25 * 3600));
    w.sql(
        "UPDATE question_ext SET agent_gone_at = ? WHERE id = ?",
        params![day_ago, old],
    );
    w.full_tick();
    assert_eq!(w.failed(), [new]);
    assert!(w.store.get(old).unwrap().unwrap().acknowledged_at.is_none()); // still for check_answers
}

fn own_cli_answer(qid: i64, answer: &str) -> Vec<Value> {
    let cmd = format!(
        "B=~/.claude/plugins/data/herdr-inbox-herdr-inbox/venv-0.10.4/bin/inbox\n$B answer {qid} \"{answer}\""
    );
    vec![
        tool_use("Bash", json!({"command": cmd}), "toolu_x"),
        tool_result(
            "toolu_x",
            &format!("Q{qid} answered, not delivered: no live agent in pane w1:p4\n"),
        ),
    ]
}

#[test]
fn reconcile_never_counts_an_answer_an_older_session_gave_itself() {
    let mut w = World::new();
    let answer = "Answer Text 105 (with #1, a; and a / slash)";
    let q = w.filed("w1:p4", NEW, "Question Text 105", answer);
    w.as_stored(q, GONE_P48, false);
    let mut records = vec![
        json!({"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW}),
        user("back"),
    ];
    records.extend(own_cli_answer(q, answer));
    w.write(OLD, &records);
    w.record_session(OLD);
    w.write(NEW, &[json!({"type": "ai-title", "aiTitle": "x"})]);
    w.full_tick();
    assert!(w.store.get(q).unwrap().unwrap().acknowledged_at.is_none());
    assert_eq!(w.failed(), [q]);
}

#[test]
fn a_mere_id_mention_with_a_short_answer_is_not_evidence() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Short one?", "yes");
    w.as_stored(q, GONE_P48, false);
    w.write(NEW, &[user(&format!("about Q{q}: yes, later"))]);
    w.full_tick();
    assert_eq!(w.failed(), [q]);
}

#[test]
fn r1_b2_a_short_answer_never_matches_inside_a_word() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Question Text 108", "Yes");
    w.as_stored(q, GONE_P48, false);
    let text = format!("Q{q} (Question Text 108) is still open. Yesterday's build is green");
    w.write(NEW, &[
        json!({"type": "assistant", "timestamp": now(), "message": {"content": [{"type": "text", "text": text}]}}),
        tool_result("t", &format!("Q{q} Question Text 108 Yesterday's build is green")),
    ]);
    w.full_tick();
    assert_eq!(w.failed(), [q]);
}

#[test]
fn r1_b3_reconcile_never_reads_an_unrelated_conversation_in_a_reused_pane() {
    let mut w = World::new();
    let other = "bbbbbbbb-0000-4000-8000-00000000000b";
    let q = w.filed("w1:p1", crate::A, "Question Text 107", "Answer Text 107");
    w.as_stored(q, "agent gone: conversation changed in pane w1:p1", false);
    w.fake.set(|k| k.agents = agents(&[("w1:p1", other)]));
    let text = w.batch_text(&[q]);
    w.write(
        other,
        &[
            tool_result("t", &format!("Operator answered Q{q}: Answer Text 107.")),
            user(&text),
        ],
    );
    w.full_tick();
    assert!(w.store.get(q).unwrap().unwrap().acknowledged_at.is_none());
    assert_eq!(w.failed(), [q]);
}

#[test]
fn r1_check_answers_output_in_a_related_session_is_evidence() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Short one?", "yes");
    w.as_stored(q, GONE_P48, false);
    let body =
        json!({"answers": [{"id": q, "question": "Short one?", "answer": "yes"}]}).to_string();
    w.write(
        NEW,
        &[
            tool_use(CHECK_ANSWERS, json!({}), "t"),
            tool_result("t", &body),
        ],
    );
    w.full_tick();
    assert!(w.store.get(q).unwrap().unwrap().acknowledged_at.is_some());
}

#[test]
fn r2_a_quoted_cli_line_without_the_own_command_is_not_evidence() {
    let mut w = World::new();
    let q = w.filed(
        "w1:p4",
        NEW,
        "Quoted elsewhere?",
        "a long answer value here",
    );
    w.as_stored(q, GONE_P48, false);
    let out = format!("$ inbox answer {q} x\nQ{q} answered, pending: busy: the agent is working");
    w.write(
        NEW,
        &[
            tool_use(
                "Bash",
                json!({"command": "herdr pane read w1:p6 --source recent"}),
                "u1",
            ),
            tool_result("u1", &out),
        ],
    );
    w.full_tick();
    assert_eq!(w.failed(), [q]);
}

#[test]
fn r2_json_the_agent_printed_itself_is_not_evidence() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Echoed json?", "yes");
    w.as_stored(q, GONE_P48, false);
    let body = json!({"id": q, "answer": "yes"}).to_string();
    w.write(
        NEW,
        &[
            tool_use("Bash", json!({"command": format!("echo '{body}'")}), "u2"),
            tool_result("u2", &body),
        ],
    );
    w.full_tick();
    assert_eq!(w.failed(), [q]);
}

#[test]
fn r3_only_a_real_successful_inbox_answer_invocation_counts() {
    let cases = [
        (
            "grep -n \"answer {n}\" ~/Code/herdr-inbox/worker.log",
            "1234: Q{n} answered, not delivered: no live agent in pane w1:p4",
        ),
        (
            "gh pr comment 44 -R LeaveMyYard/herdr-inbox --body \"run inbox answer {n} x to reproduce\"",
            "https://github.com/x/1\nQ{n} answered, pending: busy",
        ),
        (
            "inbox answer {n} x; grep 'Q{n}' ~/.local/state/herdr-inbox/worker.log",
            "Q{n} is not open\nQ{n} answered, delivered to w1:p1",
        ),
    ];
    for (command, result) in cases {
        let mut w = World::new();
        let q = w.filed(
            "w1:p4",
            NEW,
            "Anchored own answer?",
            "a long answer value here",
        );
        w.as_stored(q, GONE_P48, false);
        let n = q.to_string();
        let cmd = command.replace("{n}", &n);
        w.write(
            NEW,
            &[
                tool_use("Bash", json!({"command": cmd}), "u9"),
                tool_result("u9", &result.replace("{n}", &n)),
            ],
        );
        w.full_tick();
        assert_eq!(w.failed(), [q], "{command}");
    }
}

#[test]
fn r3_the_real_invocation_forms_count() {
    let forms = [
        "B=~/.claude/plugins/data/herdr-inbox-herdr-inbox/venv-0.10.4/bin/inbox\n$B answer {n} \"x\"",
        "cd ~/Code/herdr-inbox && uv run inbox answer Q{n} \"x\"",
        "/home/u/.venv/bin/inbox answer {n} 'x'",
        "python3 -m herdr_inbox.cli answer {n} x",
    ];
    for form in forms {
        let mut w = World::new();
        let q = w.filed("w1:p4", NEW, "Real own answer?", "a long answer value here");
        w.as_stored(q, GONE_P48, false);
        let out = format!(
            "inbox: herdr ('agent', 'get') failed: x\nQ{q} answered, not delivered: no live agent in pane w1:p4\n"
        );
        let cmd = form.replace("{n}", &q.to_string());
        w.write(
            NEW,
            &[
                tool_use("Bash", json!({"command": cmd}), "u8"),
                tool_result("u8", &out),
            ],
        );
        w.full_tick();
        assert!(
            w.store.get(q).unwrap().unwrap().acknowledged_at.is_some(),
            "{form}"
        );
    }
}

#[test]
fn a_reference_that_landed_alone_is_delivered_but_never_seen() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Question Text 107", "Answer Text 107");
    w.as_stored(q, NOT_VERIFIED, false);
    let reference = w.reference_text(q);
    w.write(NEW, &[user(&reference)]);
    w.record_session(NEW);
    w.full_tick();
    let r = w.store.get(q).unwrap().unwrap();
    assert!(r.delivered_at.is_some() && r.verified_at.is_some(), "{r:?}");
    assert!(r.acknowledged_at.is_none(), "{r:?}"); // check_answers must still return it
}

#[test]
fn a_landing_older_than_the_answer_is_not_this_answer() {
    let mut w = World::new();
    let q = w.filed("w1:p4", NEW, "Question Text 108", "Answer Text 108");
    w.as_stored(q, NOT_VERIFIED, false);
    let text = w.batch_text(&[q]);
    let mut earlier = user(&text); // ids restart in a recreated database
    earlier["timestamp"] = json!(now_minus(Duration::from_secs(3600)));
    w.write(NEW, &[earlier]);
    w.record_session(NEW);
    w.full_tick();
    untouched(&w, q);
    assert_eq!(w.failed(), [q]);
}

#[test]
fn the_failed_pile_is_reconciled_for_this_server_and_for_none_only() {
    let mut w = World::new();
    let other = w.filed("w1:p4", NEW, "Question Text 109", "Answer Text 109");
    let none = w.filed("w1:p4", NEW, "Question Text 110", "Answer Text 110");
    w.store.set_server(other, Some("/other.sock")).unwrap();
    w.store.set_server(none, None).unwrap();
    let lines: Vec<Value> = [other, none]
        .iter()
        .map(|q| user(&w.batch_text(&[*q])))
        .collect();
    for q in [other, none] {
        w.as_stored(q, NOT_VERIFIED, false);
    }
    w.write(NEW, &lines);
    w.record_session(NEW);
    w.full_tick();
    untouched(&w, other);
    assert!(w.store.get(none).unwrap().unwrap().verified_at.is_some());
}
