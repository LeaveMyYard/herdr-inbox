use super::*;
use crate::store::Store;

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

#[test]
fn the_offline_note_is_pythons() {
    use crate::clock::local_hhmm;
    use std::time::{Duration, UNIX_EPOCH};

    let since = "2026-10-18T09:05:03+00:00"; // 1_792_314_303
    let at = |secs: u64| UNIX_EPOCH + Duration::from_secs(1_792_314_303 + secs);
    let failed = |ok: Option<&str>| Machine {
        last_error: Some("Reason 123".into()),
        last_ok_at: ok.map(str::to_string),
        ..Machine::default()
    };
    assert_eq!(Machine::default().offline_note(at(0)), None);
    let healed = Machine {
        last_error: Some(String::new()),
        ..failed(Some(since))
    };
    assert_eq!(healed.offline_note(at(0)), None);
    assert_eq!(
        failed(None).offline_note(at(0)).as_deref(),
        Some("not reached yet")
    );
    let clock = local_hhmm(1_792_314_303);
    // Python: f"{hours:.0f} h" if hours >= 1 else f"{hours * 60:.0f} min"
    for (secs, age) in [
        (0, "0 min"),
        (90, "2 min"),
        (3599, "60 min"),
        (3600, "1 h"),
        (5400, "2 h"),
        (9000, "2 h"),
        (3 * 3600, "3 h"),
    ] {
        assert_eq!(
            failed(Some(since)).offline_note(at(secs)),
            Some(format!("offline since {clock} ({age})")),
            "{secs}"
        );
    }
}

#[test]
fn clearing_an_outbox_row_needs_it_clearable_at_that_moment() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_migrating(
        &dir.path().join("inbox.db"),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let insert = |id: &str, state: &str, expires: &str| {
        s.db()
            .execute(
                "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, created_at, \
                 expires_at, state) VALUES (?, 'm1', 'dd', 1, 'k', 'answer', 'c', ?, ?)",
                [id, expires, state],
            )
            .unwrap();
    };
    insert("late", "queued", "2020-01-01T00:00:00+00:00");
    insert("waits", "queued", "2040-01-01T00:00:00+00:00");
    insert("kept", "queued", "2020-01-01T00:00:00+00:00");
    let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
    let mut clock = || now;
    let state = |id: &str| -> (String, Option<String>, Option<String>) {
        s.db()
            .query_row(
                "SELECT state, result, sent_at FROM remote_outbox WHERE request_id = ?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    };
    assert!(s.clear_outbox("late", true, &mut clock).unwrap());
    assert_eq!(
        state("late"),
        ("seen".into(), Some("expired".into()), Some(iso(now)))
    );
    assert!(!s.clear_outbox("waits", true, &mut clock).unwrap());
    assert_eq!(state("waits"), ("queued".into(), None, None));
    // `expire` is the model's: without it a queued row is not settled
    assert!(s.clear_outbox("kept", false, &mut clock).unwrap());
    assert_eq!(state("kept"), ("queued".into(), None, None));
    assert!(s.db().is_autocommit(), "the transaction is closed");
}
