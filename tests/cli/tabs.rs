//! The tab prefix as Python tests/test_tabs.py drives it: a tab's `❓N`
//! follows its open questions, the operator's label is recovered and adopted,
//! marks never stack. herdr is an in-memory set of tabs.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use herdr_agent_kit::plugin_env::config::ConfigFile;
use herdr_inbox::service::answer::{answer, close};
use herdr_inbox::store::Store;
use herdr_inbox::surface::Surface;
use herdr_inbox::surface::markers::Markers;
use herdr_inbox::tabs::label::marks_for;

use crate::common::migrated;

#[derive(Default)]
struct Tabs {
    labels: BTreeMap<String, String>,
    renames: Vec<(String, String)>,
}

impl Surface for Tabs {
    fn set_open_count(&mut self, _: &str, _: i64) {}
    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        let tab = |p: &str, t: &str| (p.to_string(), Some(t.to_string()));
        Some(vec![
            tab("w1:p1", "w1:t1"),
            tab("w1:p3", "w1:t1"),
            tab("w1:p2", "w1:t2"),
        ])
    }
    fn tab_label(&mut self, tab: &str) -> Option<String> {
        self.labels.get(tab).cloned()
    }
    fn rename_tab(&mut self, tab: &str, label: &str) -> Result<(), String> {
        self.renames.push((tab.into(), label.into()));
        self.labels.insert(tab.into(), label.into());
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

struct Rig {
    dir: tempfile::TempDir,
    db: PathBuf,
    store: Store,
    tabs: Tabs,
    config: ConfigFile,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let db = migrated(dir.path());
        let store = Store::open(&db, Duration::from_secs(5)).unwrap();
        let mut tabs = Tabs::default();
        tabs.labels.insert("w1:t1".into(), "Billing API".into());
        tabs.labels.insert("w1:t2".into(), "Docs site".into());
        std::fs::create_dir_all(dir.path().join("cfg")).unwrap();
        let config = ConfigFile::in_dir(dir.path().join("cfg"));
        Rig {
            dir,
            db,
            store,
            tabs,
            config,
        }
    }

    fn set_config(&self, text: &str) {
        std::fs::write(self.dir.path().join("cfg/config.toml"), text).unwrap();
    }

    /// A question asked from `pane` (Python `service.ask`, whose markers follow).
    fn ask(&mut self, pane: &str, q: &str) -> i64 {
        self.store
            .db()
            .execute(
                "INSERT INTO questions (created_at, pane_id, question, norm, status) \
                 VALUES ('2026-10-21T10:00:00+00:00', ?, ?, ?, 'open')",
                [pane, q, q],
            )
            .unwrap();
        let id = self.store.db().last_insert_rowid();
        self.markers(pane);
        id
    }

    fn with_markers<T>(&mut self, f: impl FnOnce(&Store, &mut Markers<'_>) -> T) -> T {
        let mut warn = |m: &str| panic!("warned: {m}");
        let mut markers = Markers {
            store: &self.store,
            db_path: &self.db,
            herdr: &mut self.tabs,
            config: &mut self.config,
            lock_wait: Duration::from_secs(1),
            warn: &mut warn,
        };
        f(&self.store, &mut markers)
    }

    fn markers(&mut self, pane: &str) {
        self.with_markers(|_, m| m.update(Some(pane)));
    }

    fn label(&self, tab: &str) -> &str {
        &self.tabs.labels[tab]
    }
}

#[test]
fn prefix_added_updated_and_restored() {
    let mut rig = Rig::new();
    let a = rig.ask("w1:p1", "One?");
    assert_eq!(rig.label("w1:t1"), "❓1 Billing API");
    let b = rig.ask("w1:p3", "Two?"); // another pane in the same tab
    assert_eq!(rig.label("w1:t1"), "❓2 Billing API");
    rig.with_markers(|s, m| answer(s, a, "yes", None, m).unwrap());
    assert_eq!(rig.label("w1:t1"), "❓1 Billing API");
    rig.with_markers(|s, m| close(s, b, "dismissed", None, m).unwrap());
    assert_eq!(rig.label("w1:t1"), "Billing API");
    assert_eq!(rig.label("w1:t2"), "Docs site", "the other tab untouched");
    let want: Vec<(String, String)> = [
        "❓1 Billing API",
        "❓2 Billing API",
        "❓1 Billing API",
        "Billing API",
    ]
    .iter()
    .map(|l| ("w1:t1".to_string(), l.to_string()))
    .collect();
    assert_eq!(rig.tabs.renames, want);
}

#[test]
fn no_rename_when_label_already_right() {
    let mut rig = Rig::new();
    rig.ask("w1:p1", "One?");
    rig.tabs.renames.clear();
    rig.markers("w1:p1");
    assert!(rig.tabs.renames.is_empty());
}

#[test]
fn operator_rename_meanwhile_is_adopted() {
    let mut rig = Rig::new();
    let a = rig.ask("w1:p1", "One?");
    let b = rig.ask("w1:p1", "Two?");
    rig.tabs.labels.insert("w1:t1".into(), "Billing v2".into());
    rig.with_markers(|s, m| answer(s, a, "ok", None, m).unwrap());
    assert_eq!(rig.label("w1:t1"), "❓1 Billing v2");
    rig.tabs
        .labels
        .insert("w1:t1".into(), "❓1 Billing v3".into());
    rig.with_markers(|s, m| close(s, b, "dismissed", None, m).unwrap());
    assert_eq!(rig.label("w1:t1"), "Billing v3");
}

#[test]
fn unnamed_tab_is_left_alone_by_default() {
    let mut rig = Rig::new();
    rig.tabs.labels.insert("w1:t1".into(), "6".into());
    rig.ask("w1:p1", "One?");
    assert_eq!(rig.label("w1:t1"), "6");
    assert!(rig.tabs.renames.is_empty());
    rig.set_config("prefix_unnamed_tabs = true\n");
    rig.markers("w1:p1");
    assert_eq!(rig.label("w1:t1"), "❓1 6");
}

#[test]
fn changing_marks_never_stacks_prefixes() {
    let mut rig = Rig::new();
    rig.set_config("tab_prefix = \"Q\"\n");
    rig.ask("w1:p1", "One?");
    assert_eq!(rig.label("w1:t1"), "Q1 Billing API");
    for mark in ["❓", "X", "Q", "❓"] {
        rig.set_config(&format!("tab_prefix = \"{mark}\"\n"));
        rig.markers("w1:p1");
        assert_eq!(rig.label("w1:t1"), format!("{mark}1 Billing API"));
    }
    rig.tabs
        .labels
        .insert("w1:t1".into(), "❓1 Q1 X1 Billing API".into()); // an older build's stack heals
    rig.markers("w1:p1");
    assert_eq!(rig.label("w1:t1"), "❓1 Billing API");
    assert_eq!(
        marks_for(rig.store.db(), Some("w1:t1")).unwrap(),
        vec!["Q", "X", "❓"]
    );
}

#[test]
fn a_disabled_prefix_does_no_tab_work() {
    let mut rig = Rig::new();
    rig.set_config("tab_prefix = \"\"\n");
    rig.ask("w1:p1", "One?");
    assert!(rig.tabs.renames.is_empty());
    assert_eq!(rig.label("w1:t1"), "Billing API");
}
