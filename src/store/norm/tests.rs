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
