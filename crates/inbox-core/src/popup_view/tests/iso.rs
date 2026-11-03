//! The stored timestamps read back.

use crate::popup_view::iso::unix_seconds;

#[test]
fn pythons_isoformat_reads_back() {
    assert_eq!(
        unix_seconds("2026-10-17T12:00:00+00:00"),
        Some(1_792_238_400)
    );
    assert_eq!(unix_seconds("1970-01-01T00:00:00+00:00"), Some(0));
    assert_eq!(unix_seconds("2000-02-29T00:00:00Z"), Some(951_782_400));
}

#[test]
fn offsets_fractions_and_a_space_separator() {
    assert_eq!(
        unix_seconds("2026-10-17T14:00:00+02:00"),
        Some(1_792_238_400)
    );
    assert_eq!(
        unix_seconds("2026-10-17T11:30:00-00:30"),
        Some(1_792_238_400)
    );
    assert_eq!(
        unix_seconds("2026-10-17 12:00:00.123456+00:00"),
        Some(1_792_238_400)
    );
    assert_eq!(unix_seconds("2026-10-17T12:00:00"), Some(1_792_238_400));
}

#[test]
fn anything_else_does_not_parse() {
    for bad in [
        "",
        "yesterday",
        "2026-13-01T00:00:00",
        "2026-10-17T12:00:00~01:00",
        "2026-1x-17T12:00:00",
    ] {
        assert_eq!(unix_seconds(bad), None, "{bad}");
    }
}
