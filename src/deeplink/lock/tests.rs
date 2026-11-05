use super::*;

#[test]
fn a_held_lock_reads_as_a_running_popup() {
    let d = tempfile::tempdir().unwrap();
    let state = d.path().join("state");
    assert!(!popup_is_running(&state), "no directory: no popup");
    let held = hold_popup_lock(&state).unwrap();
    assert!(popup_is_running(&state));
    assert!(
        hold_popup_lock(&state).is_none(),
        "a second popup runs without it"
    );
    drop(held);
    assert!(!popup_is_running(&state));
}
