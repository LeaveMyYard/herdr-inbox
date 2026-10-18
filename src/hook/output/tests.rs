use super::*;

#[test]
fn the_output_is_pythons_json_dumps() {
    // json.dumps({"hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": s}}) in Python
    let s = "[inbox] Q7 answered — é\u{1F600} \"q\"\\\t\n\r\u{8}\u{c}\u{1}\u{7f}";
    assert_eq!(
        hook_output("Stop", s),
        r#"{"hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": "[inbox] Q7 answered \u2014 \u00e9\ud83d\ude00 \"q\"\\\t\n\r\b\f\u0001\u007f"}}"#
    );
}
