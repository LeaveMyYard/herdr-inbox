//! The resumed-ancestor replay (tests/fixtures/ancestor-replay.json, synthetic,
//! in the shape of herdr-inbox #53's incident): six answers asked by the
//! continuation NEW, typed into the OLDER session herdr had resumed in the
//! same pane, where the agent rejected them; a reconcile that counted
//! ancestors as proof then marked them verified. Replayed through whole
//! worker ticks, in every state such rows are seen in (and as an old
//! server's "typed = seen", for the unverified re-check), with the older
//! session's transcript holding the typed batch, its own `check_answers`
//! result and its own CLI answers: no row is ever verified, delivered or
//! seen, and nothing is typed into the older session again. The fixture's
//! times are moved so the typing was an hour ago.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_agent_kit::agent_session::claude::wire::ts;
use herdr_inbox::clock::iso_seconds;
use rusqlite::params;
use serde_json::{Value, json};

use crate::World;

const FIXTURE: &str = include_str!("../fixtures/ancestor-replay.json");

/// The incident's `at`, moved by `shift` seconds (ISO, as the store writes it).
fn moved(at: &str, shift: f64) -> String {
    let t = ts(&Value::String(at.into())).unwrap().secs() + shift;
    iso_seconds(UNIX_EPOCH + Duration::from_secs_f64(t))
}

/// The fourth state: an old server's "typed = seen" (Python
/// `_reconcile_unverified`'s signature), re-checked by the unverified pass.
const SIGNATURE: &str = "(an old server's typed = seen)";

fn replay(state: &str) {
    let f: Value = serde_json::from_str(FIXTURE).unwrap();
    let (old, new, pane) = (
        f["old"].as_str().unwrap(),
        f["new"].as_str().unwrap(),
        f["pane"].as_str().unwrap(),
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let typed = ts(&f["typed_at"]).unwrap().secs();
    let shift = now - 3600.0 - typed;
    let continued = moved(f["continued_at"].as_str().unwrap(), shift);
    let typed_at = moved(f["typed_at"].as_str().unwrap(), shift);
    let asked = moved(f["continued_at"].as_str().unwrap(), shift + 3600.0);
    let answered = moved(f["typed_at"].as_str().unwrap(), shift - 600.0);

    let signed = herdr_inbox::clock::now_minus(Duration::from_secs(60));
    let mut w = World::new();
    let mut ids = Vec::new();
    for row in f["rows"].as_array().unwrap() {
        let filed = w.filed(
            pane,
            new,
            row["question"].as_str().unwrap(),
            row["answer"].as_str().unwrap(),
        );
        let id = row["id"].as_i64().unwrap();
        w.sql(
            "UPDATE questions SET id = ?, created_at = ?, answered_at = ? WHERE id = ?",
            params![id, asked, answered, filed],
        );
        w.sql(
            "UPDATE question_ext SET id = ?, group_id = ? WHERE id = ?",
            params![id, id, filed],
        );
        w.as_stored(id, state, false);
        if state == SIGNATURE {
            // an old server typed it and called it seen in one go, a minute ago
            w.sql(
                "UPDATE questions SET delivery_error = NULL, delivered_at = ?, acknowledged_at = ? WHERE id = ?",
                params![signed, signed, id],
            );
        }
        // still retryable where the state allows it: the retry must not type it again either
        w.sql(
            "UPDATE questions SET delivery_attempts = 1 WHERE id = ?",
            params![id],
        );
        ids.push(id);
    }
    let batch = w.batch_text(&ids);
    let answers: Vec<Value> = f["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| json!({"id": r["id"], "answer": r["answer"]}))
        .collect();
    let mut records = vec![
        json!({"type": "user", "timestamp": continued, "message": {"role": "user", "content": "earlier"}}),
        json!({"type": "continued-in", "timestamp": continued, "sessionId": old, "continuedInSessionId": new}),
        json!({"type": "cost-state", "sessionId": old}),
        // herdr resumed OLD: the typed batch is its first record, and the agent there
        // fetched the answers and answered them itself
        json!({"type": "user", "timestamp": typed_at, "message": {"role": "user", "content": batch}}),
        json!({"type": "assistant", "timestamp": typed_at, "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "ca", "name": "mcp__plugin_herdr-inbox_herdr-inbox__check_answers", "input": {}}]}}),
        json!({"type": "user", "timestamp": typed_at, "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "ca", "content": json!({"answers": answers}).to_string()}]}}),
    ];
    for id in &ids {
        records.push(json!({"type": "assistant", "timestamp": typed_at, "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": format!("own{id}"), "name": "Bash", "input": {"command": format!("inbox answer {id} \"x\"")}}]}}));
        records.push(json!({"type": "user", "timestamp": typed_at, "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": format!("own{id}"), "content": format!("Q{id} answered, delivered to {pane}")}]}}));
    }
    w.write(old, &records);
    w.record_session(old);
    w.write(
        new,
        &[json!({"type": "ai-title", "aiTitle": "x", "sessionId": new})],
    );
    w.record_session(new);
    w.fake.set(|k| {
        k.status = "idle".into();
        k.session = Some(old.into()); // the pane runs the resumed OLD
        k.agents = json!([{"pane_id": pane, "tab_id": "w1:t1", "agent": "claude", "agent_status": "idle",
                           "agent_session": {"value": old}}]);
    });
    for _ in 0..3 {
        w.full_tick();
        w.memory.reconciled_at.clear(); // no back-off: every tick looks again
        w.memory.revive_checked.clear();
    }
    for id in &ids {
        let q = w.store.get(*id).unwrap().unwrap();
        assert!(
            q.verified_at.is_none(),
            "Q{id} verified in state {state:?}: {q:?}"
        );
        assert!(
            q.delivered_at.is_none() && q.acknowledged_at.is_none(),
            "Q{id} in state {state:?}: {q:?}"
        );
    }
    assert!(
        w.fake.prompts().is_empty(),
        "typed into the older session again ({state:?})"
    );
}

#[test]
fn the_replayed_rows_are_never_marked_verified() {
    let f: Value = serde_json::from_str(FIXTURE).unwrap();
    for state in f["states"].as_array().unwrap() {
        replay(state.as_str().unwrap());
    }
    replay(SIGNATURE);
}
