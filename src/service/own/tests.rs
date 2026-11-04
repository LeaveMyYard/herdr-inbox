use std::fs;
use std::time::{Duration, SystemTime};

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;

use super::*;
use crate::store::sessions::record_session;

const OLD: &str = "synthetic_91-0000-4000-8000-000000000001";
const NEW: &str = "synthetic_92-0000-4000-8000-000000000002";

struct World {
    dir: tempfile::TempDir,
    store: Store,
    files: Transcripts,
}

/// OLD continued into NEW, which asked Q1.
fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("projects/-r");
    fs::create_dir_all(&project).unwrap();
    let old = project.join(format!("{OLD}.jsonl"));
    fs::write(
        &old,
        format!(
            "{{\"type\": \"user\", \"message\": {{\"role\": \"user\", \"content\": \"before\"}}}}\n\
             {{\"type\": \"continued-in\", \"sessionId\": \"{OLD}\", \"continuedInSessionId\": \"{NEW}\", \
             \"timestamp\": \"2026-10-16T11:22:22Z\"}}\n"
        ),
    )
    .unwrap();
    fs::write(
        project.join(format!("{NEW}.jsonl")),
        "{\"type\": \"ai-title\"}\n",
    )
    .unwrap();
    let store =
        Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    record_session(store.db(), OLD, Some(&old.display().to_string())).unwrap();
    store
        .db()
        .execute(
            "INSERT INTO questions (id, created_at, pane_id, session_id, question, norm, status) \
             VALUES (1, '2026-10-16T12:00:00+00:00', 'w1:p1', ?, 'q?', 'q', 'open')",
            [NEW],
        )
        .unwrap();
    let files = Transcripts::new(Some(ProjectsRoot::new(dir.path().join("projects"))));
    World { dir, store, files }
}

fn own(w: &World, session: Option<&str>, pane: Option<&str>) -> bool {
    let q = w.store.get(1).unwrap().unwrap();
    answered_by_own_conversation(&w.store, &w.files, w.dir.path(), Me { session, pane }, &q)
        .unwrap()
}

#[test]
fn the_asker_and_its_older_session_before_the_move_are_its_own_conversation() {
    let w = world();
    assert!(own(&w, Some(NEW), Some("w1:p1")));
    assert!(
        own(&w, Some(OLD), Some("w1:p1")),
        "OLD's process runs in NEW now (D6)"
    );
    assert!(
        !own(&w, Some("77777777-0000-4000-8000-000000000007"), None),
        "another agent"
    );
    assert!(
        !own(&w, None, Some("w1:p1")),
        "the operator's shell has no session"
    );
    assert!(!own(&w, Some(""), None));
}

#[test]
fn a_resumed_older_session_is_another_conversation() {
    // Python test_older_conversation::test_the_cli_answer_from_a_resumed_older_session_is_not_the_askers_own
    let w = world();
    let starts = w.dir.path().join("starts");
    fs::create_dir_all(&starts).unwrap();
    let marker = starts.join(OLD);
    fs::write(&marker, "w1:p1").unwrap();
    let after = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_149_742 + 60);
    fs::File::options()
        .write(true)
        .open(&marker)
        .unwrap()
        .set_modified(after)
        .unwrap();
    assert!(
        !own(&w, Some(OLD), Some("w1:p1")),
        "herdr resumed OLD after the move: OLD runs itself"
    );
    assert!(
        own(&w, Some(OLD), Some("w1:p9")),
        "a resume in another pane never applies here"
    );
}

#[test]
fn a_question_without_an_asking_session_is_nobodys_own() {
    let w = world();
    w.store
        .db()
        .execute_batch("UPDATE questions SET session_id = NULL")
        .unwrap();
    assert!(!own(&w, Some(NEW), None));
}
