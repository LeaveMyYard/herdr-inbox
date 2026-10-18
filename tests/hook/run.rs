//! Running the hook binary in a scratch world.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::common::{A, migrated};

pub struct World {
    pub dir: tempfile::TempDir,
    pub db: PathBuf,
    pub transcript: PathBuf,
}

pub struct Ran {
    pub code: i32,
    pub stdout: String,
}

impl World {
    pub fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let db = migrated(dir.path());
        let project = dir.path().join("claude").join("projects").join("-r");
        std::fs::create_dir_all(&project).unwrap();
        let transcript = project.join(format!("{A}.jsonl"));
        std::fs::write(&transcript, "").unwrap();
        World {
            dir,
            db,
            transcript,
        }
    }

    pub fn payload(&self, extra: serde_json::Value) -> String {
        let mut p = serde_json::json!({"session_id": A, "transcript_path": self.transcript.to_str().unwrap()});
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        p.to_string()
    }

    /// The hook with `stdin`; `pane`: HERDR_PANE_ID (None: outside herdr).
    pub fn hook_in(&self, event: &str, stdin: &str, pane: Option<&str>) -> Ran {
        self.hook_env(event, stdin, pane, &[])
    }

    /// [`World::hook_in`] with extra environment variables.
    pub fn hook_env(
        &self,
        event: &str,
        stdin: &str,
        pane: Option<&str>,
        extra: &[(&str, &Path)],
    ) -> Ran {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_herdr-inbox"));
        cmd.args(["hook", event])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.dir.path())
            .env("HERDR_INBOX_DB", &self.db)
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .env("CLAUDE_CONFIG_DIR", self.dir.path().join("claude"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(pane) = pane {
            cmd.env("HERDR_PANE_ID", pane);
        }
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        // a hook that decides before reading stdin (outside herdr, nested) exits first: EPIPE
        if let Err(e) = child.stdin.take().unwrap().write_all(stdin.as_bytes()) {
            assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "{e}");
        }
        let out = child.wait_with_output().unwrap();
        assert!(
            out.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ran {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8(out.stdout).unwrap(),
        }
    }

    pub fn hook(&self, event: &str, stdin: &str) -> Ran {
        self.hook_in(event, stdin, Some("w1:p1"))
    }

    pub fn state(&self) -> &Path {
        self.dir.path()
    }
}

impl Ran {
    /// The additionalContext of a hook that printed, checking the shape; None when silent.
    pub fn context(&self, event: &str) -> Option<String> {
        assert_eq!(self.code, 0);
        if self.stdout.trim().is_empty() {
            return None;
        }
        assert!(
            self.stdout.ends_with('\n') && self.stdout.matches('\n').count() == 1,
            "{:?}",
            self.stdout
        );
        let v: serde_json::Value = serde_json::from_str(&self.stdout).unwrap();
        let out = &v["hookSpecificOutput"];
        assert_eq!(v.as_object().unwrap().len(), 1);
        assert_eq!(out["hookEventName"], event);
        Some(out["additionalContext"].as_str().unwrap().to_owned())
    }
}
