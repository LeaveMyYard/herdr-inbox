//! The pane a moved conversation runs in now (Python `service.session_pane`).

use std::fs;
use std::time::Duration;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{AgentInfo, ServerId};
use herdr_inbox::claude::Transcripts;
use herdr_inbox::delivery::run::{Ctx, Unreadable, session_pane};
use herdr_inbox::store::Store;
use serde_json::json;

use crate::common::migrated;

const ASKER: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const OLD: &str = "00000001-0000-4000-8000-0000000000a1";

/// #100 review L1: a pane whose herdr session continued into the asker's is
/// found through that session's recorded transcript (Python passes
/// `store.session_transcript(reported)`), even one the projects folder does
/// not list.
#[test]
fn a_pane_is_found_through_its_sessions_recorded_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let recorded = dir.path().join("elsewhere.jsonl");
    fs::write(
        &recorded,
        format!(
            "{}\n",
            json!({"type": "continued-in", "sessionId": OLD, "continuedInSessionId": ASKER})
        ),
    )
    .unwrap();
    let store = Store::open(&migrated(dir.path()), Duration::from_secs(5)).unwrap();
    store
        .db()
        .execute(
            "INSERT INTO sessions (session_id, transcript_path) VALUES (?, ?)",
            rusqlite::params![OLD, recorded.to_str().unwrap()],
        )
        .unwrap();
    let agents: Vec<AgentInfo> = serde_json::from_value(json!([
        {"pane_id": "w1:p2", "tab_id": "w1:t2", "agent": "claude", "agent_status": "idle",
         "agent_session": {"kind": "id", "value": OLD}}
    ]))
    .unwrap();
    let herdr = Herdr::new(ServerId::new(dir.path().join("no.sock")));
    let files = Transcripts::new(Some(ProjectsRoot::new(projects)));
    let (mut notify, mut unreadable) = (|_: &str, _: &str| {}, Unreadable::default());
    let ctx = Ctx {
        store: &store,
        herdr: &herdr,
        files: &files,
        state_dir: dir.path(),
        notify: &mut notify,
        unreadable: &mut unreadable,
        verify_wait: Duration::ZERO,
    };
    assert_eq!(
        session_pane(&ctx, Some(ASKER), &agents).unwrap(),
        Some(("w1:p2".to_string(), Some("w1:t2".to_string())))
    );
}
