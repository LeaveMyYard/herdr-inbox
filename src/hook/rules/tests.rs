use super::*;

/// The Python hook's source, while the Python package exists (deleted at cutover, P8).
const PYTHON: &str = include_str!("../../herdr_inbox/hook.py");

/// The text of a Python `NAME = """…"""` constant.
fn python_const(name: &str) -> &'static str {
    let start = PYTHON.find(&format!("{name} = \"\"\"")).unwrap() + name.len() + 6;
    let end = start + PYTHON[start..].find("\"\"\"").unwrap();
    &PYTHON[start..end]
}

#[test]
fn the_rules_are_pythons_word_for_word() {
    assert_eq!(SESSION_RULES, python_const("SESSION_RULES"));
    assert_eq!(SUBAGENT_RULES, python_const("SUBAGENT_RULES"));
    assert!(PYTHON.contains(
        "\"\\nYour questions already with the operator (do not ask them again; \"\n                \"call check_answers for answers you have not seen):\\n\""
    ));
    assert_eq!(
        PENDING_HEADING,
        "\nYour questions already with the operator (do not ask them again; call check_answers for answers you have not seen):\n"
    );
}

#[test]
fn the_rules_are_short() {
    // Python test_hook::test_rules_are_short
    assert!(SESSION_RULES.split_whitespace().count() < 250);
}
