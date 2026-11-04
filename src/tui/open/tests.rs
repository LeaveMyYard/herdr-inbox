use super::*;

#[test]
fn the_operators_pane_is_the_invocation_then_the_variable_then_herdrs_focus() {
    // test_race.py::test_focus_pane_comes_from_the_plugin_context
    let asked = std::cell::Cell::new(false);
    let focused = || {
        asked.set(true);
        Some("w9:p9".to_string())
    };
    assert_eq!(
        focus_pane(Some("w1:p1".into()), Some("w1:p2".into()), focused),
        Some("w1:p1".into())
    );
    assert!(!asked.get(), "herdr is asked only when nothing else says");
    assert_eq!(
        focus_pane(Some(String::new()), Some("w1:p2".into()), || None),
        Some("w1:p2".into())
    );
    assert_eq!(
        focus_pane(None, None, || Some("w9:p9".into())),
        Some("w9:p9".into())
    );
    assert_eq!(focus_pane(None, Some(String::new()), || None), None);
}
