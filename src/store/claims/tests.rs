use super::*;

#[test]
fn only_a_young_claim_is_live() {
    assert!(is_live(&format!("sending:{}:abcd1234", now())));
    assert!(!is_live(&format!(
        "sending:{}:abcd1234",
        now_minus(CLAIM_STALE + Duration::from_secs(1))
    )));
    assert!(
        !is_live("sending:2000-01-01T00:00:00+00:00:deadbeef"),
        "a stale claim"
    );
    assert!(!is_live("pending: busy: working"));
    assert!(!is_live(""));
}
