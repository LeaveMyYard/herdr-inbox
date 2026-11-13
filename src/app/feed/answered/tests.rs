use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::params;
use serde_json::{Value, json};

use super::{PER_ASKER, rows};
use crate::store::Store;
use crate::store::question::normalize_options;
use crate::store::questions::NewQuestion;

/// 2026-01-02T10:00:00+00:00.
const WALL: u64 = 1767348000;

fn wall() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(WALL)
}

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    (dir, s)
}

/// A question from `pane` with options A, B and C, answered `answer` / `detail` at `at`.
fn answered(s: &Store, pane: &str, answer: &str, detail: Option<Value>, at: &str) -> i64 {
    let options = normalize_options(&json!(["Option A", "Option B", "Option C"])).unwrap();
    let q = s
        .insert(&NewQuestion {
            question: "Question Text 345",
            options,
            pane_id: Some(pane),
            asked_by: Some("Agent 1"),
            header: Some("Header 1"),
            ..NewQuestion::default()
        })
        .unwrap();
    s.set_answer(q.id, answer, detail.as_ref()).unwrap();
    s.db()
        .execute(
            "UPDATE questions SET answered_at = ? WHERE id = ?",
            params![at, q.id],
        )
        .unwrap();
    q.id
}

const AT: &str = "2026-01-02T09:00:00+00:00";

#[test]
fn a_row_names_the_picked_labels_and_nothing_the_operator_typed() {
    let (_d, s) = store();
    let detail = json!({"choices": ["Option C", "Option Z", "Option A"],
                        "notes": {"Option A": "Note Text 1"}, "text": "Answer Text 567"});
    let id = answered(&s, "w1:p1", "Answer Text 567 secret-like", Some(detail), AT);
    s.set_tool_use_id(id, "toolu_01AbCdEf").unwrap();
    let got = rows(&s, wall()).unwrap();
    assert_eq!(got.len(), 1);
    let row = &got[0];
    assert_eq!(
        row,
        &json!({
            "key": format!("answered:{id}"), "kind": "answered", "question_id": id, "group": id,
            "title": null, "header": "Header 1", "asked_by": "Agent 1",
            "created_at": row["created_at"], "answered_at": AT,
            // in option order, and only the question's own labels
            "choices": ["Option A", "Option C"], "typed": true, "delivered": false,
            "tool_use_id": "toolu_01AbCdEf",
            "asker": {"host": "herdr-tui", "pane": "w1:p1", "terminal": null, "tab_label": null, "cwd": null},
        })
    );
    let text = row.to_string();
    for typed in [
        "Answer Text",
        "Note Text",
        "secret-like",
        "Option Z",
        "Question Text",
    ] {
        assert!(!text.contains(typed), "{typed} leaked: {text}");
    }
}

#[test]
fn an_answer_without_parts_is_a_label_only_when_it_is_exactly_one() {
    let (_d, s) = store();
    let label = answered(&s, "w1:p1", "Option B", None, AT);
    let typed = answered(&s, "w1:p1", "Option B, but later", None, AT);
    let blank_text = answered(
        &s,
        "w1:p1",
        "Option A",
        Some(json!({"choices": ["Option A"], "notes": {}, "text": "  "})),
        AT,
    );
    let noted = answered(
        &s,
        "w1:p1",
        "Option A",
        Some(json!({"choices": ["Option A"], "notes": {"Option A": "Note Text 1"}, "text": null})),
        AT,
    );
    let got = rows(&s, wall()).unwrap();
    let by = |id: i64| got.iter().find(|r| r["question_id"] == id).unwrap();
    assert_eq!(
        (&by(label)["choices"], &by(label)["typed"]),
        (&json!(["Option B"]), &json!(false))
    );
    assert_eq!(
        (&by(typed)["choices"], &by(typed)["typed"]),
        (&json!([]), &json!(true))
    );
    assert!(!by(typed).to_string().contains("later"));
    assert_eq!(
        (&by(blank_text)["choices"], &by(blank_text)["typed"]),
        (&json!(["Option A"]), &json!(false)),
        "blank text is no typing"
    );
    assert_eq!(
        (&by(noted)["choices"], &by(noted)["typed"]),
        (&json!(["Option A"]), &json!(true)),
        "a note alone is typing"
    );
    assert!(
        !by(noted).to_string().contains("Note Text"),
        "the note is never shown"
    );
}

#[test]
fn only_answers_of_the_last_day_newest_first() {
    let (_d, s) = store();
    let old = answered(&s, "w1:p1", "Option A", None, "2026-01-01T09:59:59+00:00");
    let edge = answered(&s, "w1:p1", "Option A", None, "2026-01-01T10:00:00+00:00");
    let newer = answered(&s, "w1:p1", "Option A", None, AT);
    let open = s
        .insert(&NewQuestion {
            question: "Question Text 346",
            pane_id: Some("w1:p1"),
            ..NewQuestion::default()
        })
        .unwrap();
    let dismissed = s
        .insert(&NewQuestion {
            question: "Question Text 347",
            pane_id: Some("w1:p1"),
            ..NewQuestion::default()
        })
        .unwrap();
    s.set_status(dismissed.id, "dismissed", Some("Note Text 1"))
        .unwrap();
    let ids: Vec<i64> = rows(&s, wall())
        .unwrap()
        .iter()
        .map(|r| r["question_id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![newer, edge]);
    assert!(!ids.contains(&old) && !ids.contains(&open.id) && !ids.contains(&dismissed.id));
}

#[test]
fn at_most_twenty_per_asker_and_each_asker_has_its_own() {
    let (_d, s) = store();
    let mut first = Vec::new();
    for minute in 0..25 {
        let at = format!("2026-01-02T09:{minute:02}:00+00:00");
        first.push(answered(&s, "w1:p1", "Option A", None, &at));
    }
    let other = answered(&s, "w1:p2", "Option B", None, "2026-01-02T08:00:00+00:00");
    let got = rows(&s, wall()).unwrap();
    let of = |pane: &str| -> Vec<i64> {
        got.iter()
            .filter(|r| r["asker"]["pane"] == pane)
            .map(|r| r["question_id"].as_i64().unwrap())
            .collect()
    };
    let newest: Vec<i64> = first.iter().rev().take(PER_ASKER).copied().collect();
    assert_eq!(of("w1:p1"), newest);
    assert_eq!(of("w1:p2"), vec![other]);
}

#[test]
fn an_answer_the_agent_has_is_delivered() {
    let (_d, s) = store();
    let sent = answered(&s, "w1:p1", "Option A", None, AT);
    let seen = answered(&s, "w1:p1", "Option A", None, AT);
    let waiting = answered(&s, "w1:p1", "Option A", None, AT);
    s.db()
        .execute(
            "UPDATE questions SET delivered_at = ? WHERE id = ?",
            params![AT, sent],
        )
        .unwrap();
    s.acknowledge(&[seen]).unwrap();
    let got = rows(&s, wall()).unwrap();
    let delivered =
        |id: i64| got.iter().find(|r| r["question_id"] == id).unwrap()["delivered"].clone();
    assert_eq!(delivered(sent), json!(true));
    assert_eq!(delivered(seen), json!(true));
    assert_eq!(delivered(waiting), json!(false));
}

/// A core asker has no pane: its terminal tells two agents apart, and an `unknown` asker (no pane,
/// no terminal) is a third. A key by pane alone would put all of them in one bucket of 20.
#[test]
fn pane_less_askers_are_told_apart_by_terminal() {
    use crate::store::hosts::CoreSeat;
    let (_d, s) = store();
    let core = |terminal: &str, minute: usize| {
        let id = answered(
            &s,
            "",
            "Option A",
            None,
            &format!("2026-01-02T09:{minute:02}:00+00:00"),
        );
        s.db()
            .execute("UPDATE questions SET pane_id = NULL WHERE id = ?", [id])
            .unwrap();
        let seat = CoreSeat {
            terminal: terminal.into(),
            machine: None,
        };
        s.set_question_host(id, &seat).unwrap();
        id
    };
    let t3: Vec<i64> = (0..PER_ASKER + 1).map(|m| core("t3", 10 + m)).collect();
    let t4 = core("t4", 5);
    // neither a pane nor a terminal: `unknown`, a third asker
    let unknown = core("t5", 1);
    s.db()
        .execute("DELETE FROM question_host WHERE id = ?", [unknown])
        .unwrap();
    let got = rows(&s, wall()).unwrap();
    let of = |host: &str, terminal: Option<&str>| -> Vec<i64> {
        got.iter()
            .filter(|r| r["asker"]["host"] == host && r["asker"]["terminal"].as_str() == terminal)
            .map(|r| r["question_id"].as_i64().unwrap())
            .collect()
    };
    let newest: Vec<i64> = t3.iter().rev().take(PER_ASKER).copied().collect();
    assert_eq!(of("herdr-core", Some("t3")), newest);
    assert_eq!(of("herdr-core", Some("t4")), vec![t4]);
    assert_eq!(of("unknown", None), vec![unknown]);
}
