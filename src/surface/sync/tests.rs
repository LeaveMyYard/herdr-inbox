use std::time::Duration;

use herdr_agent_kit::plugin_env::config::ConfigFile;

use super::*;
use crate::store::Store;
use crate::surface::Surface;

#[derive(Default)]
struct Herdr {
    calls: Vec<String>,
}

impl Surface for Herdr {
    fn set_open_count(&mut self, pane: &str, count: i64) {
        self.calls.push(format!("token {pane} {count}"));
    }
    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        Some(vec![
            ("p1".into(), Some("t1".into())),
            ("p1".into(), Some("t1".into())),
        ])
    }
    fn all_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        Some(vec![
            ("t1".into(), Some("!1 web".into())),
            ("t2".into(), None),
        ])
    }
    fn tab_label(&mut self, _: &str) -> Option<String> {
        Some("!1 web".into())
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
}

#[test]
fn removing_prefixes_strips_a_custom_mark_once_per_pane_and_tab() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("inbox.db");
    let store = Store::open_migrating(&path, Duration::from_secs(5)).unwrap();
    store
        .db()
        .execute_batch("INSERT INTO tab_decor (tab_id, mark) VALUES ('t1', '!')")
        .unwrap();
    let mut herdr = Herdr::default();
    let mut config = ConfigFile::default();
    let mut warn = |m: &str| panic!("{m}");
    Markers {
        store: &store,
        db_path: &path,
        herdr: &mut herdr,
        config: &mut config,
        lock_wait: Duration::from_secs(1),
        warn: &mut warn,
    }
    .sync(true)
    .unwrap();
    assert_eq!(
        herdr.calls,
        vec!["token p1 0", "rename t1 web"],
        "a pane listed twice: one token"
    );
}
