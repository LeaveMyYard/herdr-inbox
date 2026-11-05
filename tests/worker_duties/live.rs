//! The live duties' tick result: what keeps the worker alive (Python
//! test_alerts.py `test_the_worker_stays_alive_while_herdr_reports_agents_...`,
//! test_remote_sync.py `test_the_worker_stays_alive_while_a_remote_is_enabled`):
//! pending answers of this server, agents herdr reports (their blocked state
//! is watched) and an enabled saved machine (whose pulls the remote thread
//! runs). herdr's `machine list` is a script; ssh is a path that does not
//! exist, so the remote thread's pulls fail without leaving the test's dir.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use herdr_agent_kit::plugin_env::config::ConfigFile;
use herdr_inbox::clock::now_minus;
use herdr_inbox::worker::duties::live::LiveDuties;
use herdr_inbox::worker::live::Duties;
use herdr_inbox::worker::remote::RemoteEnv;
use herdr_inbox::worker::remote::ssh::SshEnv;
use herdr_inbox::worker::{Waits, WorkerEnv};
use serde_json::json;

use crate::World;

impl World {
    fn duties(&self, machines: &str) -> LiveDuties {
        self.duties_on(machines, &self.db)
    }

    /// How often `machine list` was asked for.
    fn machine_lists(&self) -> usize {
        fs::read_to_string(self.dir.path().join("herdr-cli.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains("machine list"))
            .count()
    }

    fn duties_on(&self, machines: &str, db: &std::path::Path) -> LiveDuties {
        let d = self.dir.path();
        let bin = d.join("herdr-cli");
        let log = d.join("herdr-cli.log");
        fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\ncat <<'EOF'\n{machines}\nEOF\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        let herdr = self.herdr.clone().with_bin(bin);
        let env = WorkerEnv {
            db: db.to_path_buf(),
            server: self.server.clone().into(),
            revision: herdr_inbox::store::revision::MIGRATION_REVISION,
            version: "0.0.0".into(),
            waits: Waits::default(),
            spawn_env: Vec::new(),
            no_worker: true,
        };
        let remote = RemoteEnv {
            db: self.db.clone(),
            herdr: herdr.clone(),
            ssh: SshEnv {
                program: d.join("no-ssh").into(),
                runtime_dir: None,
                home: d.to_path_buf(),
            },
            config_dir: None,
        };
        let files = herdr_inbox::claude::Transcripts::new(Some(
            herdr_agent_kit::agent_session::claude::io::ProjectsRoot::new(self.projects.clone()),
        ));
        LiveDuties::new(env, herdr, files, ConfigFile::in_dir(d.join("cfg")), remote)
    }
}

#[test]
fn nothing_pending_no_agents_no_machines_lets_the_worker_idle() {
    let w = World::new();
    let mut d = w.duties("[]");
    d.migrate();
    let t = d.tick();
    assert_eq!((t.pending, t.keep_alive), (0, false));
    assert_eq!(d.pending_now(), 0);
}

#[test]
fn the_worker_stays_alive_while_herdr_reports_agents() {
    let w = World::new();
    w.fake.set(|k| {
        k.agents = json!([{"pane_id": "w1:p9", "agent": "claude", "agent_status": "working"}])
    });
    let t = w.duties("[]").tick();
    assert_eq!((t.pending, t.keep_alive), (0, true));
}

#[test]
fn the_worker_stays_alive_while_a_remote_is_enabled() {
    let w = World::new();
    let machines = r#"[{"id": "m1", "label": "box", "target": "box.lan", "enabled": true}]"#;
    let t = w.duties(machines).tick();
    assert_eq!((t.pending, t.keep_alive), (0, true));
}

#[test]
fn own_pending_rows_keep_it_alive_and_are_counted_after_the_lock_goes() {
    let mut w = World::new();
    w.pending_answer("blocked");
    let mut d = w.duties("[]");
    assert_eq!(d.tick().pending, 1);
    assert_eq!(d.pending_now(), 1);
}

#[test]
fn other_servers_rows_are_not_pending_here_after_the_lock_goes_either() {
    let mut w = World::new();
    let qid = w.ask_on(("w1:p1", "w1:t1"), Some("/tmp/other.sock"));
    w.answer(qid, "blocked"); // pending, but on another herdr server
    let mut d = w.duties("[]");
    assert_eq!(d.tick().pending, 0);
    assert_eq!(d.pending_now(), 0);
}

#[test]
fn the_remote_thread_starts_once_and_the_machine_list_is_reused() {
    let w = World::new();
    let machines = r#"[{"id": "m1", "label": "box", "target": "box.lan", "enabled": true}]"#;
    let mut d = w.duties(machines);
    for _ in 0..3 {
        d.tick();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while w.machine_lists() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(500)); // a second thread would ask too
    assert_eq!(w.machine_lists(), 2, "the duties' and the one thread's");
}

#[test]
fn a_database_that_cannot_open_keeps_the_worker_but_starts_nobody() {
    let w = World::new();
    let mut d = w.duties_on("[]", w.dir.path()); // a directory
    assert_eq!(d.tick().pending, 1);
    assert_eq!(d.pending_now(), 0);
}

#[test]
fn a_failed_tick_keeps_the_worker() {
    let w = World::new();
    let mut d = w.duties("[]");
    w.sql("DROP TABLE changes", []); // the tick's trim is fatal
    assert_eq!(d.tick().pending, 1);
}

#[test]
fn the_open_re_checks_unverified_acknowledgements() {
    let mut w = World::new();
    let qid = w.answered();
    w.signature(qid, &now_minus(Duration::from_secs(60)));
    w.landed(qid);
    let mut d = w.duties("[]");
    d.pending_now(); // the only pass it gets is its open's
    assert!(w.store.get(qid).unwrap().unwrap().verified_at.is_some());
}
