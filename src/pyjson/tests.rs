use serde_json::json;

use super::*;

#[test]
fn values_are_written_as_python_writes_them() {
    // json.dumps({"choices": ["eu", "api, v2"], "notes": {"web": "n ü"}, "text": None, "n": 3, "b": True})
    let v = json!({"choices": ["eu", "api, v2"], "notes": {"web": "n ü"}, "text": null, "n": 3, "b": true});
    assert_eq!(
        dumps(&v),
        r#"{"choices": ["eu", "api, v2"], "notes": {"web": "n \u00fc"}, "text": null, "n": 3, "b": true}"#
    );
    assert_eq!(dumps(&json!([])), "[]");
    assert_eq!(dumps(&json!({})), "{}");
    assert_eq!(
        string("\u{1F642}\u{7f}\u{1}"),
        r#""\ud83d\ude42\u007f\u0001""#
    );
}

#[test]
fn a_lone_surrogate_reads_as_python_reads_it_as_closely_as_a_rust_string_can() {
    // json.loads('"a\\ud83db"') == "a\ud83db" in Python; here U+FFFD
    assert_eq!(loads(r#""a\ud83db""#).unwrap(), json!("a\u{FFFD}b"));
    assert_eq!(
        loads(r#"{"n": "\udc00\ud83d"}"#).unwrap(),
        json!({"n": "\u{FFFD}\u{FFFD}"})
    );
    assert_eq!(
        loads(r#"["\ud83d"]"#).unwrap(),
        json!(["\u{FFFD}"]),
        "at the end"
    );
    // a pair, escapes around it and an escaped backslash before a "u" stay as they are
    assert_eq!(
        loads(r#"["\ud83d\ude42", "\\ud83d", "\"\ud83d\n"]"#).unwrap(),
        json!(["\u{1F642}", "\\ud83d", "\"\u{FFFD}\n"])
    );
    assert!(
        loads(r#"{"a": }"#).is_err(),
        "broken JSON stays an error, as in Python"
    );
}
