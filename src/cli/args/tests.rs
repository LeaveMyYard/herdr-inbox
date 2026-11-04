use super::*;

#[test]
fn a_question_ref_loses_one_q_right_before_the_number() {
    // Python deeplink.question_ref (#55)
    for (given, want) in [
        ("qnap:Q12", "qnap:12"),
        ("Q12", "12"),
        ("q12", "12"),
        ("QQ12", "Q12"),
        ("12", "12"),
        ("a:b:q3", "a:b:3"),
        ("", ""),
        ("Q", ""),
    ] {
        assert_eq!(question_ref(given), want, "{given}");
    }
}

#[test]
fn a_question_id_is_a_local_ref_as_int_reads_it() {
    assert_eq!(question_id("Q12"), Some(12));
    assert_eq!(question_id("q٣"), Some(3));
    assert_eq!(question_id(" q3"), None, "the Q must start the ref");
    assert_eq!(question_id("Q 3 "), Some(3), "int() strips");
    assert_eq!(question_id("Q-3"), Some(-3));
    assert_eq!(question_id("QQ12"), None);
    assert_eq!(question_id("qnap:Q12"), None);
    assert_eq!(
        question_id("Q99999999999999999999"),
        None,
        "Python overflows in SQLite"
    );
}
