use std::time::Duration;

use herdr_agent_kit::plugin_env::config::ConfigFile;

use super::*;
use crate::surface::Surface;

struct Quiet;

impl Surface for Quiet {
    fn set_open_count(&mut self, _: &str, _: i64) {}
    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        None
    }
    fn tab_label(&mut self, _: &str) -> Option<String> {
        None
    }
    fn rename_tab(&mut self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
}

#[test]
fn typed_text_is_a_detail_unless_blank() {
    assert_eq!(
        text_detail("  go ahead \n"),
        Some(json!({"choices": [], "notes": {}, "text": "go ahead"}))
    );
    assert_eq!(text_detail(" \t\u{1f}"), None);
    assert_eq!(
        dumps(&text_detail("x").unwrap()),
        r#"{"choices": [], "notes": {}, "text": "x"}"#,
        "Python's json.dumps of make_detail"
    );
}

#[test]
fn a_stored_detail_reads_as_its_picks_notes_and_text() {
    let d = json!({"choices": ["Postgres", "a, b"], "notes": {"Postgres": "keep it", "a, b": ""}, "text": " more "});
    assert_eq!(answer_text(&d), r#"Postgres (note: keep it), "a, b", more"#);
    assert_eq!(answer_text(&json!({"text": "only"})), "only");
    assert_eq!(answer_text(&json!({"choices": null, "notes": null})), "");
}

#[test]
fn a_draft_read_with_loss_is_never_answered() {
    // Python: the lone surrogate reaches SQLite in the answer, which raises (UnicodeEncodeError):
    // the drafts before it stay answered, it stays open, nothing after it is answered
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let lossy = r#"{"choices": [], "notes": {}, "text": "a\udc80b"}"#;
    let fine = r#"{"choices": [], "notes": {}, "text": "ok"}"#;
    store
        .db()
        .execute_batch(&format!(
            "INSERT INTO questions (id, created_at, question, norm, status) VALUES \
             (1, 'x', 'a?', 'a', 'open'), (2, 'x', 'b?', 'b', 'open'), (3, 'x', 'c?', 'c', 'open'); \
             INSERT INTO question_ext (id, group_id, draft_json) VALUES \
             (1, 1, '{lossy}'), (2, 2, '{fine}'), (3, 3, '{fine}');"
        ))
        .unwrap();
    let mut config = ConfigFile::default();
    let mut warn = |_: &str| {};
    let mut markers = Markers {
        store: &store,
        db_path: &path,
        herdr: &mut Quiet,
        config: &mut config,
        lock_wait: Duration::ZERO,
        warn: &mut warn,
    };
    store
        .db()
        .execute_batch(
            "INSERT INTO questions (id, created_at, question, norm, status) VALUES (4, 'x', 'd?', 'd', 'open'); \
             INSERT INTO question_ext (id, group_id, draft_json) VALUES (4, 4, '[1]');",
        )
        .unwrap();
    assert!(
        submit(&store, &[4], None, &mut markers).is_err(),
        "a draft that is no object"
    );
    assert_eq!(store.get(4).unwrap().unwrap().status, "open");
    assert!(submit(&store, &[2, 1, 3], None, &mut markers).is_err());
    let status = |id: i64| store.get(id).unwrap().unwrap().status;
    assert_eq!(
        (status(2), status(1), status(3)),
        ("answered".into(), "open".into(), "open".into())
    );
    assert_eq!(
        submit(&store, &[3, 3], None, &mut markers).unwrap().len(),
        1,
        "answered once"
    );
}

#[test]
fn closing_needs_an_open_question() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let mut config = ConfigFile::default();
    let mut warn = |_: &str| {};
    let mut markers = Markers {
        store: &store,
        db_path: &path,
        herdr: &mut Quiet,
        config: &mut config,
        lock_wait: Duration::ZERO,
        warn: &mut warn,
    };
    assert!(!close(&store, 7, "dismissed", None, &mut markers).unwrap());
    assert_eq!(answer(&store, 7, "x", None, &mut markers).unwrap(), None);
}
