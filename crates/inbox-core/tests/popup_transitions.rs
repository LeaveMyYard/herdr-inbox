//! The popup model replays the popup transition golden
//! (tests/fixtures/popup-transitions.json, captured from the Python popup by
//! tools/capture_popup_transitions.py): every scenario's events, at their
//! times, give the same state and the same effects, step by step.

mod popup;

use inbox_core::popup::Popup;
use popup::json;
use serde_json::Value;

const GOLDEN: &str = include_str!("../../../tests/fixtures/popup-transitions.json");

/// The keys where two states differ, with both values.
fn differences(want: &Value, got: &Value) -> Vec<String> {
    let (Some(w), Some(g)) = (want.as_object(), got.as_object()) else {
        return vec![format!("want {want}\n  got  {got}")];
    };
    let mut keys: Vec<&String> = w.keys().chain(g.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|k| w.get(*k) != g.get(*k))
        .map(|k| {
            format!(
                "{k}: want {}\n  {:width$} got  {}",
                w.get(k).unwrap_or(&Value::Null),
                "",
                g.get(k).unwrap_or(&Value::Null),
                width = k.len()
            )
        })
        .collect()
}

fn replay(scenario: &Value) -> Result<(), String> {
    let name = scenario["name"].as_str().unwrap();
    let steps = scenario["steps"].as_array().unwrap();
    let first = &steps[0];
    let (mut model, effects) = Popup::open(
        json::open(&first["event"]["open"]),
        first["at"].as_u64().unwrap(),
    );
    let mut results = vec![(effects, json::state(&model))];
    for step in &steps[1..] {
        let effects = model.handle(json::event(&step["event"]), step["at"].as_u64().unwrap());
        results.push((effects, json::state(&model)));
    }
    for (i, (step, (effects, state))) in steps.iter().zip(results).enumerate() {
        let effects = Value::Array(effects.iter().map(json::effect).collect());
        let event = match step["event"].get("key") {
            Some(k) => format!("key {k}"),
            None => step["event"]
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap()
                .clone(),
        };
        let at = &step["at"];
        if effects != step["effects"] {
            return Err(format!(
                "{name} step {i} ({event} at {at}): effects\n  want {}\n  got  {effects}",
                step["effects"]
            ));
        }
        if state != step["state"] {
            let diff = differences(&step["state"], &state).join("\n  ");
            return Err(format!(
                "{name} step {i} ({event} at {at}): state\n  {diff}"
            ));
        }
    }
    Ok(())
}

#[test]
fn every_scenario_replays_as_the_python_popup_ran_it() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(golden["schema"], 1);
    let scenarios = golden["scenarios"].as_array().unwrap();
    assert!(
        scenarios.len() >= 70,
        "the golden lost scenarios: {}",
        scenarios.len()
    );
    let failures: Vec<String> = scenarios.iter().filter_map(|s| replay(s).err()).collect();
    assert!(
        failures.is_empty(),
        "{} of {} scenarios differ:\n\n{}",
        failures.len(),
        scenarios.len(),
        failures.join("\n\n")
    );
}

#[test]
fn the_delays_are_the_pythons() {
    use inbox_core::popup::{GO_PAUSE, GRACE, RELEASE, ROW_ARM, VIEW_ARM};
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let d = &golden["delays_ms"];
    assert_eq!(
        ["arm", "row_arm", "grace", "release", "go"].map(|k| d[k].as_u64().unwrap()),
        [VIEW_ARM, ROW_ARM, GRACE, RELEASE, GO_PAUSE]
    );
}
