//! Tab marks against other processes (Python tests/test_tabs.py): herdr
//! hanging never holds the database, and a count read before another
//! process's change never ends up on the tab. Each side has its own store and
//! its own herdr, as separate processes do; the tab-label lock is the real one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use herdr_agent_kit::plugin_env::config::ConfigFile;
use herdr_inbox::store::{OpenError, Store};
use herdr_inbox::surface::Surface;
use herdr_inbox::surface::markers::Markers;

use crate::common::migrated;

const PANES: [(&str, &str); 2] = [("w1:p1", "w1:t1"), ("w1:p3", "w1:t1")];

type Labels = Arc<Mutex<BTreeMap<String, String>>>;

/// herdr's tabs, shared by every process; `Hold` stalls one process's herdr.
#[derive(Clone)]
struct Herdr {
    labels: Labels,
    hold: Option<Hold>,
}

/// One stall: `entered` says herdr was reached, then it hangs until `release`.
#[derive(Clone)]
struct Hold {
    method: &'static str,
    entered: Sender<()>,
    release: Arc<Mutex<Receiver<()>>>,
}

impl Herdr {
    fn called(&mut self, method: &str) {
        if let Some(h) = self.hold.take_if(|h| h.method == method) {
            h.entered.send(()).unwrap();
            h.release.lock().unwrap().recv().unwrap();
        }
    }
}

impl Surface for Herdr {
    fn set_open_count(&mut self, _: &str, _: i64) {
        self.called("set_open_count");
    }
    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        self.called("pane_tabs");
        let pair = |(p, t): &(&str, &str)| (p.to_string(), Some(t.to_string()));
        Some(PANES.iter().map(pair).collect())
    }
    fn all_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        self.called("all_tabs");
        let labels = self.labels.lock().unwrap();
        Some(
            labels
                .iter()
                .map(|(t, l)| (t.clone(), Some(l.clone())))
                .collect(),
        )
    }
    fn tab_label(&mut self, tab: &str) -> Option<String> {
        let label = self.labels.lock().unwrap().get(tab).cloned();
        self.called("tab_label");
        label
    }
    fn rename_tab(&mut self, tab: &str, label: &str) -> Result<(), String> {
        self.called("rename_tab");
        self.labels.lock().unwrap().insert(tab.into(), label.into());
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
    fn open_popup(&mut self, _: &[(String, String)]) -> Result<(), String> {
        Ok(())
    }
    fn radar_installed(&mut self) -> bool {
        false
    }
    fn radar_config_dir(&mut self) -> Option<PathBuf> {
        None
    }
    fn reload_config(&mut self) {}
}

/// Runs `f` with one process's markers: its own store, config and herdr.
fn process<T>(db: &Path, herdr: &mut Herdr, f: impl FnOnce(&Store, &mut Markers<'_>) -> T) -> T {
    let store = Store::open(db, Duration::from_secs(5)).unwrap();
    let dir = db.parent().unwrap().join("cfg");
    let mut config = ConfigFile::in_dir(dir);
    let mut warn = |m: &str| panic!("warned: {m}");
    let mut markers = Markers {
        store: &store,
        db_path: db,
        herdr,
        config: &mut config,
        lock_wait: Duration::from_secs(10),
        warn: &mut warn,
    };
    f(&store, &mut markers)
}

fn ask(store: &Store, pane: &str) -> i64 {
    store
        .db()
        .execute(
            "INSERT INTO questions (created_at, pane_id, question, norm, status) \
             VALUES ('2026-10-21T10:00:00+00:00', ?, 'Question Title 123', 'q', 'open')",
            [pane],
        )
        .unwrap();
    store.db().last_insert_rowid()
}

fn tabs() -> Labels {
    Labels::new(Mutex::new(BTreeMap::from([(
        "w1:t1".to_string(),
        "Billing API".to_string(),
    )])))
}

/// A herdr that stalls at `method`; the sender lets it go.
fn stalled(labels: &Labels, method: &'static str) -> (Herdr, Receiver<()>, Sender<()>) {
    let (entered, reached) = channel();
    let (go, release) = channel();
    let hold = Hold {
        method,
        entered,
        release: Arc::new(Mutex::new(release)),
    };
    let herdr = Herdr {
        labels: Arc::clone(labels),
        hold: Some(hold),
    };
    (herdr, reached, go)
}

/// A marker refresh, as one process runs it.
type Refresh = dyn Fn(&mut Markers<'_>) + Sync;

/// A writer with no busy wait at all: a lock held across the herdr call fails it at once.
fn writes_at_once(db: &Path, qid: i64) -> rusqlite::Result<bool> {
    let other = Store::open(db, Duration::ZERO).map_err(|e| match e {
        OpenError::Sqlite(e) => e,
        other => panic!("{other}"),
    })?;
    let detail = serde_json::json!({"text": "Question Text 345"});
    let drafted = other.set_draft(qid, Some(&detail))?;
    other.db().execute_batch("BEGIN IMMEDIATE; ROLLBACK")?;
    Ok(drafted)
}

#[test]
fn a_hung_herdr_never_blocks_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = migrated(dir.path());
    let labels = tabs();
    let qid = ask(&Store::open(&db, Duration::from_secs(5)).unwrap(), "w1:p1");
    // every herdr call a marker refresh or the sync makes
    let update = |m: &mut Markers<'_>| m.update(Some("w1:p1"));
    let sync = |m: &mut Markers<'_>| m.sync(false).unwrap();
    let refreshes: [(&str, &Refresh); 7] = [
        ("set_open_count", &update),
        ("pane_tabs", &update),
        ("tab_label", &update),
        ("rename_tab", &update),
        ("set_open_count", &sync),
        ("all_tabs", &sync),
        ("rename_tab", &sync),
    ];
    for (method, refresh) in refreshes {
        labels
            .lock()
            .unwrap()
            .insert("w1:t1".into(), "Billing API".into());
        let (mut herdr, reached, go) = stalled(&labels, method);
        thread::scope(|s| {
            let db = &db;
            s.spawn(move || process(db, &mut herdr, |_, m| refresh(m)));
            reached.recv().unwrap();
            let wrote = writes_at_once(db, qid);
            go.send(()).unwrap(); // before any assert: a failure must never leave herdr hung
            assert_eq!(wrote, Ok(true), "herdr stalled in {method}");
        });
    }
}

#[test]
fn concurrent_updates_never_leave_a_stale_count() {
    let dir = tempfile::tempdir().unwrap();
    let db = migrated(dir.path());
    let labels = tabs();
    let mut herdr = Herdr {
        labels: Arc::clone(&labels),
        hold: None,
    };
    let first = process(&db, &mut herdr, |st, m| {
        let id = ask(st, "w1:p1");
        m.update(Some("w1:p1"));
        id
    });
    assert_eq!(labels.lock().unwrap()["w1:t1"], "❓1 Billing API");
    let store = Store::open(&db, Duration::from_secs(5)).unwrap();
    assert!(store.set_status(first, "dismissed", None).unwrap()); // count 0, marks not refreshed
    let (mut closer, reached, go) = stalled(&labels, "tab_label");
    thread::scope(|s| {
        let db = &db;
        s.spawn(move || process(db, &mut closer, |_, m| m.update(Some("w1:p1"))));
        reached.recv().unwrap(); // it has read count 0 and will rename to the bare label
        let asker = s.spawn(move || {
            process(db, &mut herdr, |st, m| {
                ask(st, "w1:p3");
                m.update(Some("w1:p3"));
            })
        });
        thread::sleep(Duration::from_millis(300)); // the asker runs meanwhile, or waits for the lock
        go.send(()).unwrap();
        asker.join().unwrap();
    });
    assert_eq!(labels.lock().unwrap()["w1:t1"], "❓1 Billing API");
}
