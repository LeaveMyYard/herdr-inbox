use std::time::{Duration, UNIX_EPOCH};

use super::*;

#[test]
fn the_stored_format_is_pythons_isoformat_in_seconds() {
    // values printed by Python: datetime.fromtimestamp(t, timezone.utc).isoformat(timespec="seconds")
    for (t, want) in [
        (0, "1970-01-01T00:00:00+00:00"),
        (951_782_400, "2000-02-29T00:00:00+00:00"),
        (1_792_314_303, "2026-10-18T09:05:03+00:00"),
        (4_102_444_799, "2099-12-31T23:59:59+00:00"),
    ] {
        assert_eq!(
            iso_seconds(UNIX_EPOCH + Duration::from_secs(t)),
            want,
            "{t}"
        );
    }
}

#[test]
fn a_fraction_of_a_second_is_dropped_not_rounded() {
    let t = UNIX_EPOCH + Duration::from_millis(1_792_314_303_999);
    assert_eq!(iso_seconds(t), "2026-10-18T09:05:03+00:00");
}

#[test]
fn now_minus_is_earlier_and_compares_as_text() {
    let (a, b) = (now_minus(Duration::from_secs(60)), now());
    assert!(a < b, "{a} {b}");
}
