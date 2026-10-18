use serde_json::Value;

use super::*;

const GOLDEN: &str = include_str!("../../../tests/fixtures/store-questions.json");

#[test]
fn normalize_and_clean_title_are_pythons() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    for t in golden["texts"].as_array().unwrap() {
        let text = t["text"].as_str().unwrap();
        assert_eq!(
            normalize(text),
            t["normalize"].as_str().unwrap(),
            "normalize({text:?})"
        );
        assert_eq!(
            clean_title(Some(text)).as_deref(),
            t["clean_title"].as_str(),
            "clean_title({text:?})"
        );
    }
    assert_eq!(clean_title(None), None);
}

/// The Unicode version both follow (see the module doc): Python 3.14's, 16.0.
/// Python 3.14.7 gives these; 3.12 (15.0) has every one unassigned (`"a b"`).
#[test]
fn unicode_16_as_python_3_14() {
    assert_eq!(unicode_general_category::UNICODE_VERSION, (16, 0, 0));
    for (c, norm, title) in [
        ('\u{2FFC}', "a b", "a\u{2FFC}b"),           // 15.1, So
        ('\u{31EF}', "a b", "a\u{31EF}b"),           // 15.1, So
        ('\u{1C89}', "a\u{1C8A}b", "a\u{1C89}b"),    // 16.0, Lu
        ('\u{A7CB}', "a\u{0264}b", "a\u{A7CB}b"),    // 16.0, Lu
        ('\u{10D4A}', "a\u{10D4A}b", "a\u{10D4A}b"), // 16.0, Lo
        ('\u{A7D2}', "a b", "a b"), // 17.0: unassigned in 16.0, std would lowercase it
        ('\u{A7D4}', "a b", "a b"), // 17.0
    ] {
        let text = format!("a{c}b");
        assert_eq!(normalize(&text), norm, "normalize U+{:04X}", c as u32);
        assert_eq!(
            clean_title(Some(&text)).as_deref(),
            Some(title),
            "clean_title U+{:04X}",
            c as u32
        );
    }
}
