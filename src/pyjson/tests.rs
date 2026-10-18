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
