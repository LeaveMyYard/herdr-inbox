use std::time::Duration;

use super::*;
use crate::store::Store;

#[derive(Default)]
struct Spy {
    calls: Vec<String>,
}

impl Surface for Spy {
    fn set_open_count(&mut self, pane: &str, count: i64) {
        self.calls.push(format!("token {pane} {count}"));
    }
    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        self.calls.push("pane_tabs".into());
        Some(vec![("w1:p1".into(), Some("w1:t1".into()))])
    }
    fn tab_label(&mut self, tab: &str) -> Option<String> {
        self.calls.push(format!("tab_label {tab}"));
        Some("web".into())
    }
    fn rename_tab(&mut self, tab: &str, label: &str) -> Result<(), String> {
        self.calls.push(format!("rename {tab} {label}"));
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
    fn open_popup(&mut self, _: &[(String, String)]) -> Result<(), String> {
        Ok(())
    }
    fn radar_installed(&mut self) -> bool {
        false
    }
    fn radar_config_dir(&mut self) -> Option<std::path::PathBuf> {
        None
    }
    fn reload_config(&mut self) {}
    fn all_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        None
    }
}

fn store(dir: &Path) -> (Store, std::path::PathBuf) {
    let path = dir.join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    store
        .db()
        .execute_batch(
            "INSERT INTO questions (created_at, pane_id, question, norm, status) \
             VALUES ('2026-10-21T10:00:00+00:00', 'w1:p1', 'q?', 'q', 'open')",
        )
        .unwrap();
    std::fs::create_dir_all(dir.join("cfg")).unwrap();
    std::fs::write(dir.join("cfg/config.toml"), "tab_prefix = \"!\"\n").unwrap();
    (store, path)
}

#[test]
fn a_busy_tab_lock_skips_the_label_but_not_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(dir.path());
    let held = crate::tabs::lock::marker_lock(&path, Duration::ZERO, &mut |_| {}).unwrap();
    assert!(held.is_some());
    let mut spy = Spy::default();
    let mut config = ConfigFile::in_dir(dir.path().join("cfg"));
    let mut warned = Vec::new();
    let mut warn = |m: &str| warned.push(m.to_string());
    {
        let mut markers = Markers {
            store: &store,
            db_path: &path,
            herdr: &mut spy,
            config: &mut config,
            lock_wait: Duration::from_millis(100),
            warn: &mut warn,
        };
        markers.update(Some("w1:p1"));
        markers.update(None);
        markers.update(Some(""));
    }
    assert_eq!(spy.calls, vec!["token w1:p1 1", "pane_tabs"]);
    assert_eq!(warned, vec![crate::tabs::lock::BUSY.to_string()]);
    drop(held);
    let mut warn = |m: &str| panic!("{m}");
    let mut markers = Markers {
        store: &store,
        db_path: &path,
        herdr: &mut spy,
        config: &mut config,
        lock_wait: Duration::from_millis(100),
        warn: &mut warn,
    };
    markers.update(Some("w1:p1"));
    assert_eq!(
        spy.calls[2..],
        [
            "token w1:p1 1",
            "pane_tabs",
            "tab_label w1:t1",
            "rename w1:t1 !1 web"
        ]
    );
}

#[test]
fn a_database_error_is_warned_about_never_raised() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(dir.path());
    store.db().execute_batch("DROP TABLE alerts").unwrap();
    let mut spy = Spy::default();
    let mut config = ConfigFile::in_dir(dir.path().join("cfg"));
    let mut warned = Vec::new();
    let mut warn = |m: &str| warned.push(m.to_string());
    Markers {
        store: &store,
        db_path: &path,
        herdr: &mut spy,
        config: &mut config,
        lock_wait: Duration::ZERO,
        warn: &mut warn,
    }
    .update(Some("w1:p1"));
    assert!(spy.calls.is_empty());
    assert_eq!(warned.len(), 1);
    assert!(
        warned[0].starts_with("could not refresh the inbox markers for w1:p1: "),
        "{warned:?}"
    );
}

#[test]
fn an_open_alert_counts_in_the_panes_token_beside_its_questions() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(dir.path()); // one open question in w1:p1
    store
        .db()
        .execute_batch(
            "INSERT INTO alerts (created_at, kind, severity, title, pane_id) \
             VALUES ('2026-10-21T10:00:00+00:00', 'agent', 'urgent', 'Question Title 123', 'w1:p1')",
        )
        .unwrap();
    let mut spy = Spy::default();
    let mut config = ConfigFile::in_dir(dir.path().join("cfg"));
    let mut warn = |m: &str| panic!("{m}");
    Markers {
        store: &store,
        db_path: &path,
        herdr: &mut spy,
        config: &mut config,
        lock_wait: Duration::from_millis(100),
        warn: &mut warn,
    }
    .update(Some("w1:p1"));
    assert_eq!(spy.calls[0], "token w1:p1 2");
}
