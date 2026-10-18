//! Claude Code transcripts in a scratch projects directory.

use std::fs;
use std::path::{Path, PathBuf};

use herdr_agent_kit::agent_session::claude::io::ProjectsRoot;
use herdr_inbox::claude::Transcripts;

pub const OLD: &str = "1cd70de7-0000-4000-8000-000000000001";
pub const NEW: &str = "c5240607-0000-4000-8000-000000000002";

pub struct Projects {
    pub dir: PathBuf,
}

impl Projects {
    pub fn new(root: &Path) -> Projects {
        let dir = root.join("projects").join("-r");
        fs::create_dir_all(&dir).unwrap();
        Projects { dir }
    }

    pub fn files(&self) -> Transcripts {
        Transcripts::new(Some(ProjectsRoot::new(self.dir.parent().unwrap())))
    }

    pub fn path(&self, session: &str) -> PathBuf {
        self.dir.join(format!("{session}.jsonl"))
    }

    /// Appends records to a session's transcript.
    pub fn write(&self, session: &str, lines: &[String]) -> PathBuf {
        let path = self.path(session);
        let mut text = fs::read_to_string(&path).unwrap_or_default();
        for l in lines {
            text.push_str(l);
            text.push('\n');
        }
        fs::write(&path, text).unwrap();
        path
    }
}

pub fn continued_in(from: &str, to: &str, ts: Option<&str>) -> String {
    let ts = ts
        .map(|t| format!(r#","timestamp":"{t}""#))
        .unwrap_or_default();
    format!(r#"{{"type":"continued-in","sessionId":"{from}","continuedInSessionId":"{to}"{ts}}}"#)
}

pub fn user(session: &str, ts: &str, content: &str) -> String {
    serde_json::json!({"type": "user", "sessionId": session, "timestamp": ts,
                       "message": {"role": "user", "content": content}})
    .to_string()
}
