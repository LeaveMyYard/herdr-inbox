use super::*;

#[test]
fn targets_are_valid_as_pythons_patterns_read_them() {
    for (kind, value, want) in [
        ("question", "12", true),
        ("question", "Q12", true),
        ("question", "q١٢", true),
        ("question", "qnap:Q12", true),
        ("question", "a.b_c-d:12", true),
        ("question", "QQ12", false),
        ("question", "a:b:12", false),
        ("question", ":12", false),
        ("question", &format!("{}:1", "m".repeat(65)), false),
        ("question", "Q", false),
        ("group", "30", true),
        ("group", "3a", false),
        ("group", "", false),
        ("pane", "w1:p1", true),
        ("pane", "a b", false),
        ("pane", "a\u{1f}b", false),
        ("pane", "", false),
        ("session", "06ACC80F-0000-4000-8000-000000000002", true),
        ("session", "nope", false),
        ("bogus", "1", false),
    ] {
        assert_eq!(valid(kind, value), want, "{kind} {value:?}");
    }
}

#[test]
fn a_target_is_named_as_the_notice_names_it() {
    assert_eq!(describe("question", "12"), "Q12");
    assert_eq!(describe("question", "qnap:Q12"), "qnap:Q12");
    assert_eq!(describe("question", "qnap:12"), "qnap:Q12");
    assert_eq!(describe("question", "box:12"), "box:Q12");
    assert_eq!(describe("question", "Q12"), "Q12");
    assert_eq!(
        describe("question", " Q30 "),
        "Q Q30 ",
        "Python strips nothing here"
    );
    assert_eq!(describe("pane", "w1:p2"), "pane w1:p2");
}
