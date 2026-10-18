use super::*;

#[test]
fn a_payload_is_read_as_python_reads_its_dict() {
    let p = Payload::parse(
        r#"{"session_id":"s","transcript_path":"/t","source":"resume","agent_id":"a1"}"#,
    )
    .unwrap();
    assert_eq!(
        p,
        Payload {
            session_id: Some("s".into()),
            transcript_path: Some("/t".into()),
            source: Some("resume".into()),
            agent_id: true
        }
    );
    for falsy in ["null", "\"\"", "0", "false", "[]", "{}"] {
        let p = Payload::parse(&format!(r#"{{"agent_id":{falsy},"session_id":""}}"#)).unwrap();
        assert!(!p.agent_id && p.session_id.is_none(), "{falsy}");
    }
    assert_eq!(Payload::parse("  \n"), Some(Payload::default()));
    assert_eq!(Payload::parse("{not json"), None);
    // valid JSON that is not an object reads as {} (Python: the bare rules at SessionStart)
    assert_eq!(Payload::parse("[1]"), Some(Payload::default()));
    assert_eq!(Payload::parse("1"), Some(Payload::default()));
}

#[test]
fn a_lone_surrogate_escape_does_not_reject_the_payload() {
    // a string cut inside an emoji, as Node writes it; Python's json.loads accepts it
    let raw = r#"{"session_id":"s","tool_response":"cut \ud83d","agent_id":""}"#;
    let p = Payload::parse(raw).expect("parsed like Python");
    assert_eq!(p.session_id.as_deref(), Some("s"));
}

#[test]
fn a_payload_deeper_than_128_levels_is_still_read() {
    // Python's json.loads reads it; a deep MCP structuredContent in tool_response
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    let raw = format!(
        r#"{{"session_id":"s","tool_response":{deep},"agent_id":[[[]]],"source":"resume"}}"#
    );
    let p = Payload::parse(&raw).expect("read");
    assert_eq!(
        (p.session_id.as_deref(), p.source.as_deref(), p.agent_id),
        (Some("s"), Some("resume"), true)
    );
    let broken = format!(r#"{{"session_id":"s","tool_response":{deep}"#);
    assert_eq!(Payload::parse(&broken), None, "still JSON or nothing");
    let not_object = format!("[{deep}]");
    assert_eq!(
        Payload::parse(&not_object),
        None,
        "a deep non-object is not read (documented)"
    );
}

#[test]
fn a_field_holding_a_lone_surrogate_reads_as_absent() {
    // Python could not write such an id to SQLite and raised before writing anything
    let p = Payload::parse(r#"{"session_id":"s\ud83d","transcript_path":"/t\udc00"}"#).unwrap();
    assert_eq!((p.session_id, p.transcript_path), (None, None));
}

#[test]
fn a_payload_deeper_than_python_reads_is_not_read() {
    let too_deep = format!(
        r#"{{"session_id":"s","x":{}{}}}"#,
        "[".repeat(200_000),
        "]".repeat(200_000)
    );
    assert_eq!(Payload::parse(&too_deep), None);
    assert_eq!(
        super::deep::depth(r#"{"a":"[[[{{","b":[[1]]}"#),
        3,
        "brackets in strings do not count"
    );
    assert_eq!(
        super::deep::depth(r#"{"a":"\"[[["}"#),
        1,
        "an escaped quote does not end the string"
    );
}
