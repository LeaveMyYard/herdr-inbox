use serde_json::json;

use super::*;

#[test]
fn radar_is_installed_when_listed_and_not_disabled() {
    let listed = |p: serde_json::Value| radar_listed(&json!({"plugins": [{"plugin_id": "x"}, p]}));
    assert!(
        listed(json!({"plugin_id": RADAR_ID})),
        "no flag: enabled, as .get(\"enabled\", True)"
    );
    assert!(listed(json!({"plugin_id": RADAR_ID, "enabled": true})));
    assert!(!listed(json!({"plugin_id": RADAR_ID, "enabled": false})));
    assert!(!listed(json!({"plugin_id": RADAR_ID, "enabled": null})));
    assert!(!listed(json!({"plugin_id": "other"})));
    assert!(!radar_listed(&json!({})));
}
