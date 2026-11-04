use super::*;

#[test]
fn ints_parse_as_python_parses_them() {
    // each checked against Python 3.13's int()
    for (s, want) in [
        ("12", Some(12)),
        ("  +12\n", Some(12)),
        ("-0", Some(0)),
        ("007", Some(7)),
        ("1_000", Some(1000)),
        ("١٢", Some(12)),
        ("𝟗", Some(9)),
        ("߁߀", Some(10)),
        ("9223372036854775807", Some(i64::MAX)),
        ("-9223372036854775808", Some(i64::MIN)),
        ("9223372036854775808", None),
        ("", None),
        ("-", None),
        ("_1", None),
        ("1_", None),
        ("1__0", None),
        ("- 1", None),
        ("1.0", None),
        ("²", None),
        ("0x10", None),
    ] {
        assert_eq!(parse(s), want, "{s:?}");
    }
}
