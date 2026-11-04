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
    fn open_popup(&mut self, _: &[(String, String)]) -> Result<(), String> {
        Ok(())
    }
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
fn a_draft_read_with_loss_goes_into_the_answer_as_stored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    let raw = r#"{"choices": [], "notes": {}, "text": "a\udc80b"}"#;
    store
        .db()
        .execute_batch(&format!(
            "INSERT INTO questions (id, created_at, question, norm, status) \
             VALUES (1, '2026-10-21T10:00:00+00:00', 'q?', 'q', 'open'); \
             INSERT INTO question_ext (id, group_id, draft_json) VALUES (1, 1, '{raw}');"
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
    let done = submit(&store, &[1, 1], None, &mut markers).unwrap();
    assert_eq!(done.len(), 1, "a question is answered once");
    let stored: String = store
        .db()
        .query_row(
            "SELECT answer_json FROM question_ext WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, raw, "never the U+FFFD the lossy read holds");
    assert_eq!(done[0].answer.as_deref(), Some("a\u{fffd}b"));
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
