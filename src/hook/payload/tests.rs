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
    assert_eq!(Payload::parse("[1]"), None);
}
