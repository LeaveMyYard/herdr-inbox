use std::fs;

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;

use super::*;

const SID: &str = "aaaaaaaa-0000-4000-8000-000000000001";

#[test]
fn a_recorded_file_wins_then_the_projects_directory_is_searched() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("projects").join("-r");
    fs::create_dir_all(&project).unwrap();
    let found = project.join(format!("{SID}.jsonl"));
    fs::write(&found, "").unwrap();
    let recorded = dir.path().join("elsewhere.jsonl");
    fs::write(&recorded, "").unwrap();
    let t = Transcripts::new(Some(ProjectsRoot::new(dir.path().join("projects"))));
    assert_eq!(t.find(SID, recorded.to_str()), Some(recorded.clone()));
    assert_eq!(t.find(SID, Some("/no/such/file")), Some(found.clone()));
    assert_eq!(
        t.find(&SID.to_uppercase(), None),
        Some(found),
        "any case in"
    );
    // a recorded file is used whatever the id; only an id is never a pattern
    assert_eq!(t.find("not-an-id", recorded.to_str()), Some(recorded));
    for bad in ["*", "../x", "", "not-an-id"] {
        assert_eq!(t.find(bad, None), None, "{bad}");
    }
    assert_eq!(Transcripts::new(None).find(SID, None), None);
}
