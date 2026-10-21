//! The screen parsers against Python's own answers
//! (tests/fixtures/herdr-screens.json, `tools/capture_goldens.py --only herdr`):
//! herdr.composer_state and herdr.panel_focused over the captured Claude
//! Code screens and the shapes the Python tests build.

use herdr_inbox::delivery::screen::{composer_state, panel_focused};
use inbox_core::deliver::Composer;
use serde_json::Value;

const GOLDEN: &str = include_str!("fixtures/herdr-screens.json");

#[test]
fn every_screen_reads_as_python_reads_it() {
    let doc: Value = serde_json::from_str(GOLDEN).unwrap();
    let rows = doc["rows"].as_array().unwrap();
    let (mut composers, mut panels) = (0, 0);
    for row in rows.iter().filter(|r| r.get("screen").is_some()) {
        let (name, screen) = (
            row["name"].as_str().unwrap(),
            row["screen"].as_str().unwrap(),
        );
        if let Some(want) = row.get("composer").and_then(Value::as_str) {
            let want = match want {
                "empty" => Composer::Empty,
                "draft" => Composer::Draft,
                _ => Composer::Unknown,
            };
            assert_eq!(composer_state(screen), want, "{name}");
            composers += 1;
        }
        if let Some(want) = row.get("panel").and_then(Value::as_bool) {
            assert_eq!(panel_focused(screen), want, "{name}");
            panels += 1;
        }
    }
    assert!(composers >= 17 && panels >= 4);
}

/// What Python's `live_agent` returned, as the `Agent` the policy reads (a
/// status Python leaves as `None` or a number is printed as `str()` does).
fn python_agent(v: &Value) -> Option<inbox_core::deliver::Agent> {
    let a = v.as_object()?;
    let text = |k: &str| a[k].as_str().map(String::from);
    Some(inbox_core::deliver::Agent {
        kind: text("agent"),
        status: match &a["status"] {
            Value::String(s) => s.clone(),
            Value::Null => "None".into(),
            other => other.to_string(),
        },
        focused: a["focused"].as_bool().unwrap(),
        session: text("session_id"),
        reported_session: text("reported_session_id"),
    })
}

#[test]
fn every_agent_record_reads_as_python_reads_it() {
    use herdr_inbox::delivery::pane::agent_from;

    let doc: Value = serde_json::from_str(GOLDEN).unwrap();
    let rows: Vec<&Value> = doc["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r.get("result").is_some())
        .collect();
    assert!(rows.len() >= 19);
    for row in rows {
        let result = Some(&row["result"]).filter(|r| !r.is_null());
        let got = agent_from(result, |reported| format!("live-of-{reported}"));
        assert_eq!(got, python_agent(&row["agent"]), "{}", row["name"]);
    }
}
