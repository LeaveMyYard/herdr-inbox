use std::collections::BTreeMap;

use super::*;

#[derive(Default)]
struct Tabs {
    labels: BTreeMap<String, String>,
    renames: Vec<(String, String)>,
    refuse: bool,
}

impl Surface for Tabs {
    fn set_open_count(&mut self, _: &str, _: i64) {}
    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        None
    }
    fn tab_label(&mut self, tab: &str) -> Option<String> {
        self.labels.get(tab).cloned()
    }
    fn rename_tab(&mut self, tab: &str, label: &str) -> Result<(), String> {
        self.renames.push((tab.into(), label.into()));
        if self.refuse {
            return Err("no".into());
        }
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

fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE tab_decor (tab_id TEXT, mark TEXT, PRIMARY KEY (tab_id, mark))")
        .unwrap();
    db
}

fn decor(db: &Connection) -> Vec<(String, String)> {
    let mut stmt = db
        .prepare("SELECT tab_id, mark FROM tab_decor ORDER BY 1, 2")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_custom_mark_is_recorded_before_it_decorates_and_dropped_once_restored() {
    let db = db();
    let mut tabs = Tabs::default();
    tabs.labels.insert("t1".into(), "Q4 roadmap".into());
    apply(&db, &mut tabs, "t1", 2, "!", || false).unwrap();
    assert_eq!(
        tabs.labels["t1"], "!2 Q4 roadmap",
        "an operator's `Q4` is not ours"
    );
    assert_eq!(decor(&db), vec![("t1".into(), "!".into())]);
    apply(&db, &mut tabs, "t1", 0, "!", || false).unwrap();
    assert_eq!(tabs.labels["t1"], "Q4 roadmap");
    assert!(decor(&db).is_empty());
}

#[test]
fn a_tab_herdr_does_not_report_is_left_alone() {
    let db = db();
    let mut tabs = Tabs::default();
    apply(&db, &mut tabs, "t9", 3, "❓", || false).unwrap();
    assert!(tabs.renames.is_empty() && decor(&db).is_empty());
}

#[test]
fn a_refused_rename_keeps_the_record() {
    let db = db();
    let mut tabs = Tabs {
        refuse: true,
        ..Tabs::default()
    };
    tabs.labels.insert("t1".into(), "web".into());
    apply(&db, &mut tabs, "t1", 1, "!", || false).unwrap();
    assert_eq!(tabs.renames, vec![("t1".into(), "!1 web".into())]);
    assert_eq!(decor(&db), vec![("t1".into(), "!".into())]);
}
