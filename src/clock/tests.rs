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

#[test]
fn hours_and_minutes_are_the_local_clocks() {
    // values printed by Python: datetime.fromtimestamp(t, timezone(timedelta(seconds=o))).strftime("%H:%M")
    for (t, offset, want) in [
        (0, 0, "00:00"),
        (1_792_314_303, 0, "09:05"),
        (1_792_314_303, 7200, "11:05"),
        (1_792_314_303, -34_200, "23:35"),
        (1_792_314_303, 19_800, "14:35"),
        (-60, 0, "23:59"),
    ] {
        assert_eq!(hhmm(t, offset), want, "{t} {offset}");
    }
}

#[test]
fn the_operators_clock_keeps_the_shape_at_any_time() {
    for t in [0, 1_792_314_303, i64::MAX] {
        let shown = local_hhmm(t);
        assert_eq!(shown.len(), 5, "{shown}");
        assert_eq!(&shown[2..3], ":", "{shown}");
    }
}
