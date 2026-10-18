use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;

use super::*;

const OLD: &str = "1cd70de7-0000-4000-8000-000000000001";
const NEW: &str = "c5240607-0000-4000-8000-000000000002";

fn world(record_at: Option<&str>) -> (tempfile::TempDir, Transcripts) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("projects").join("-r");
    fs::create_dir_all(&project).unwrap();
    let ts = record_at
        .map(|t| format!(r#","timestamp":"{t}""#))
        .unwrap_or_default();
    let line = format!(
        r#"{{"type":"continued-in","sessionId":"{OLD}","continuedInSessionId":"{NEW}"{ts}}}"#
    );
    fs::write(project.join(format!("{OLD}.jsonl")), format!("{line}\n")).unwrap();
    let files = Transcripts::new(Some(ProjectsRoot::new(dir.path().join("projects"))));
    (dir, files)
}

fn mark(state: &Path, pane: &str, at: SystemTime) {
    let starts = state.join("starts");
    fs::create_dir_all(&starts).unwrap();
    let path = starts.join(OLD);
    fs::write(&path, pane).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(at)
        .unwrap();
}

#[test]
fn a_continued_session_runs_its_continuation_unless_resumed_after_it() {
    // record at 2026-10-16T11:22:22Z = 1792149742
    let (dir, files) = world(Some("2026-10-16T11:22:22Z"));
    let state = dir.path().join("state");
    assert_eq!(
        live_session(&files, &state, OLD, Some("w1:p1")),
        NEW,
        "D6: no marker"
    );
    let record = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_149_742);
    mark(&state, "w1:p1", record - Duration::from_secs(60));
    assert_eq!(
        live_session(&files, &state, OLD, Some("w1:p1")),
        NEW,
        "a resume before the move"
    );
    mark(&state, "w1:p1", record + Duration::from_secs(60));
    assert_eq!(
        live_session(&files, &state, OLD, Some("w1:p1")),
        OLD,
        "resumed after: runs itself"
    );
    assert_eq!(
        live_session(&files, &state, &OLD.to_uppercase(), None),
        OLD,
        "no pane: the marker applies"
    );
    assert_eq!(
        live_session(&files, &state, OLD, Some("w1:p9")),
        NEW,
        "a resume in another pane never applies here"
    );
    mark(&state, "", record + Duration::from_secs(60));
    assert_eq!(
        live_session(&files, &state, OLD, Some("w1:p9")),
        OLD,
        "a marker without a pane applies anywhere"
    );
}

#[test]
fn an_undated_continuation_or_an_unknown_session_follows_d6() {
    let (dir, files) = world(None);
    let state = dir.path().join("state");
    mark(&state, "w1:p1", SystemTime::now());
    assert_eq!(
        live_session(&files, &state, OLD, Some("w1:p1")),
        NEW,
        "no record time to compare with"
    );
    assert_eq!(live_session(&files, &state, NEW, None), NEW);
    assert_eq!(live_session(&files, &state, "not-an-id", None), "not-an-id");
    assert_eq!(live_session(&files, &state, "", None), "");
}
