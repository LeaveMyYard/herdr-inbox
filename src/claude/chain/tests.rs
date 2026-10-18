use std::fs;
use std::path::Path;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;

use super::*;

fn sid(n: u32) -> String {
    format!("aaaaaaaa-0000-4000-8000-{n:012}")
}

fn continues(dir: &Path, from: &str, to: &str) {
    let line = format!(
        r#"{{"type":"continued-in","sessionId":"{from}","continuedInSessionId":"{to}","timestamp":"2026-10-16T11:22:00Z"}}"#
    );
    fs::write(dir.join(format!("{from}.jsonl")), format!("{line}\n")).unwrap();
}

#[test]
fn the_chain_follows_continuations_in_order_and_stops_at_a_cycle_or_the_hop_cap() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("p");
    fs::create_dir_all(&project).unwrap();
    let files = Transcripts::new(Some(ProjectsRoot::new(dir.path())));
    continues(&project, &sid(1), &sid(2));
    continues(&project, &sid(2), &sid(3));
    assert_eq!(
        chain(&files, &sid(1).to_uppercase(), None),
        [sid(1), sid(2), sid(3)]
    );
    continues(&project, &sid(3), &sid(1));
    assert_eq!(
        chain(&files, &sid(1), None),
        [sid(1), sid(2), sid(3)],
        "cycle-safe"
    );
    for n in 10..30 {
        continues(&project, &sid(n), &sid(n + 1));
    }
    assert_eq!(chain(&files, &sid(10), None).len(), MAX_HOPS);
    assert_eq!(chain(&files, "not-an-id", None), ["not-an-id"]);
    assert_eq!(chain(&files, "", None), Vec::<String>::new());
}

#[test]
fn the_recorded_path_is_used_for_the_first_session_only() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("recorded.jsonl");
    let line = format!(
        r#"{{"type":"continued-in","continuedInSessionId":"{}"}}"#,
        sid(2)
    );
    fs::write(&elsewhere, format!("{line}\n")).unwrap();
    let files = Transcripts::new(None);
    assert_eq!(chain(&files, &sid(1), elsewhere.to_str()), [sid(1), sid(2)]);
}
