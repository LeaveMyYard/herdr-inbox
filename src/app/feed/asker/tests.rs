use serde_json::json;

use super::asker;

#[test]
fn a_terminal_makes_a_core_asker_with_no_pane_and_no_label() {
    let a = asker(
        Some("t3".into()),
        Some("w1:p1"),
        Some("Project Alpha"),
        Some("/work"),
    );
    let want = json!({"host": "herdr-core", "pane": null, "terminal": "t3", "tab_label": null, "cwd": "/work"});
    assert_eq!(a, want);
}

#[test]
fn a_pane_makes_a_tui_asker_and_the_inboxs_prefix_goes() {
    let a = asker(None, Some("w1:p1"), Some("❓2 Project Alpha"), None);
    let want = json!({"host": "herdr-tui", "pane": "w1:p1", "terminal": null, "tab_label": "Project Alpha", "cwd": null});
    assert_eq!(a, want);
}

#[test]
fn neither_is_unknown_and_an_empty_pane_is_none() {
    assert_eq!(asker(None, Some(""), None, None)["host"], "unknown");
    assert_eq!(asker(None, None, None, None)["pane"], json!(null));
}
