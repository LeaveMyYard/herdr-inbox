use super::*;

#[test]
fn the_first_line_is_pythons() {
    // Python: service_first_line(t) for each
    assert_eq!(first_line("  a\nb  "), "a");
    assert_eq!(first_line("x\u{2028}y"), "x");
    assert_eq!(first_line("x\u{1c}y"), "x");
    assert_eq!(first_line("\r\nz"), "z");
    assert_eq!(first_line("   "), "");
    assert_eq!(first_line(&"é".repeat(300)).chars().count(), 200);
}

#[test]
fn a_string_plugin_list_fails_on_its_first_character_without_expanding_it() {
    // Python: iterating "xxx…" gives "x", and "x".get(...) raises at once
    let big = format!(
        r#"{{"result": {{"plugins": "{}"}}}}"#,
        "x".repeat(1_000_000)
    );
    assert_eq!(
        super::paths::plugin_root_for_test(&big),
        Err("'str' object has no attribute 'get'".into())
    );
    let keys = r#"{"result": {"plugins": {"a": 1}}}"#;
    assert_eq!(
        super::paths::plugin_root_for_test(keys),
        Err("'str' object has no attribute 'get'".into())
    );
    assert_eq!(
        super::paths::plugin_root_for_test(r#"{"result": {"plugins": ""}}"#),
        Ok(None)
    );
    assert_eq!(
        super::paths::plugin_root_for_test(r#"{"result": {"plugins": {}}}"#),
        Ok(None)
    );
}

#[test]
fn probe_parsing_is_linear() {
    // 200,000 distinct safe lines (about 2 MB): a scan per line (quadratic) takes minutes;
    // the bound is generous for any machine but far below that
    let text: String = (0..200_000).map(|i| format!("/p/n{i}\n")).collect();
    let start = std::time::Instant::now();
    let got = probed(&text);
    assert_eq!(got.len(), 200_000);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    let again: String = "/a/uv\n/b/uv\n/c/herdr\n".into();
    assert_eq!(
        probed(&again),
        vec![
            ("uv".to_string(), "/b/uv".to_string()),
            ("herdr".to_string(), "/c/herdr".to_string())
        ]
    );
}
