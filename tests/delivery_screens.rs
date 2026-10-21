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
    for row in rows {
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
