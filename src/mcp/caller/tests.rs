use std::cell::RefCell;
use std::collections::BTreeMap;

use super::*;

/// A scripted world: herdr's panes and agents, the continuations, the marks.
#[derive(Default)]
struct Fake {
    nested: bool,
    panes: BTreeMap<String, Pane>,
    tabs: BTreeMap<String, String>,
    agents: Vec<(String, Option<String>)>,
    /// reported session -> the session it lives in now
    continued: BTreeMap<String, String>,
    /// (reported session, pane) -> the session it lives in, as seen from that pane only
    in_pane: BTreeMap<(String, String), String>,
    marks: Vec<String>,
    marks_fail: bool,
    calls: RefCell<Vec<String>>,
}

impl World for Fake {
    fn nested(&self) -> bool {
        self.calls.borrow_mut().push("nested".into());
        self.nested
    }
    fn pane(&self, pane: &str) -> Option<Pane> {
        self.calls.borrow_mut().push(format!("pane {pane}"));
        self.panes.get(pane).cloned()
    }
    fn tab_label(&self, tab: &str) -> Option<String> {
        self.calls.borrow_mut().push(format!("tab {tab}"));
        self.tabs.get(tab).cloned()
    }
    fn agents(&self) -> Vec<(String, Option<String>)> {
        self.calls.borrow_mut().push("agents".into());
        self.agents.clone()
    }
    fn live(&self, reported: &str, pane: Option<&str>) -> String {
        if let Some(p) = pane
            && let Some(live) = self.in_pane.get(&(reported.to_owned(), p.to_owned()))
        {
            return live.clone();
        }
        self.continued
            .get(reported)
            .cloned()
            .unwrap_or_else(|| reported.to_owned())
    }
    fn marks(&self, _tab: Option<&str>) -> Result<Vec<String>, Failure> {
        if self.marks_fail {
            return Err(Failure::NotReady);
        }
        Ok(self.marks.clone())
    }
}

fn pane(tab: &str, session: Option<&str>) -> Pane {
    Pane {
        tab_id: Some(tab.into()),
        session: session.map(str::to_owned),
    }
}

fn env(pane: Option<&str>, session: Option<&str>) -> Env {
    Env {
        pane_id: pane.map(str::to_owned),
        session_id: session.map(str::to_owned),
        tab_id: None,
        cwd: Some("/r".into()),
    }
}

fn world() -> Fake {
    let mut w = Fake::default();
    w.panes.insert("w1:p1".into(), pane("w1:t1", Some("S")));
    w.tabs.insert("w1:t1".into(), "❓2 my-tab".into());
    w.marks = vec!["❓".into()];
    w
}

#[test]
fn the_pane_its_tab_and_its_session_come_from_herdr() {
    let c = resolve(&world(), &env(Some("w1:p1"), Some("ENV"))).unwrap();
    assert_eq!(
        c,
        Caller {
            pane_id: Some("w1:p1".into()),
            tab_id: Some("w1:t1".into()),
            tab_label: Some("my-tab".into()), // never our "❓2 " prefix
            session_id: Some("S".into()),
            cwd: Some("/r".into()),
        }
    );
}

#[test]
fn a_nested_claude_is_not_the_panes_agent() {
    let mut w = world();
    w.nested = true;
    let c = resolve(&w, &env(Some("w1:p1"), Some("ENV"))).unwrap();
    assert_eq!(c.pane_id, None);
    assert_eq!(c.session_id.as_deref(), Some("ENV"));
    assert_eq!((c.tab_id, c.tab_label), (None, None));
    assert_eq!(*w.calls.borrow(), ["nested"]); // herdr is never asked about the pane
}

#[test]
fn outside_herdr_nothing_is_asked() {
    let w = world();
    let c = resolve(&w, &env(None, Some("ENV"))).unwrap();
    assert_eq!((c.pane_id, c.session_id.as_deref()), (None, Some("ENV")));
    assert!(w.calls.borrow().is_empty());
}

#[test]
fn a_moved_pane_is_found_by_the_conversation_it_runs() {
    let mut w = world();
    w.panes.remove("w1:p1"); // herdr gave the moved pane a new id
    w.panes.insert("w6:p57".into(), pane("w6:t2", Some("S")));
    w.agents = vec![
        ("w2:p1".into(), Some("OTHER".into())),
        ("w6:p57".into(), Some("S".into())),
        ("w3:p1".into(), None),
    ];
    let c = resolve(&w, &env(Some("w6:p48"), Some("S"))).unwrap();
    assert_eq!(c.pane_id.as_deref(), Some("w6:p57"));
    assert_eq!(c.tab_id.as_deref(), Some("w6:t2"));
    assert_eq!(c.session_id.as_deref(), Some("S"));
}

#[test]
fn a_moved_pane_follows_continuations_on_both_sides() {
    let mut w = world();
    w.panes.clear();
    w.panes.insert("w6:p57".into(), pane("w6:t2", Some("OLD")));
    w.continued.insert("OLD".into(), "NEW".into());
    w.continued.insert("ENV".into(), "NEW".into());
    w.agents = vec![("w6:p57".into(), Some("OLD".into()))];
    let c = resolve(&w, &env(Some("w6:p48"), Some("ENV"))).unwrap();
    assert_eq!(c.pane_id.as_deref(), Some("w6:p57"));
    assert_eq!(c.session_id.as_deref(), Some("NEW")); // the live session files it
}

#[test]
fn two_candidates_or_none_keep_the_dead_pane() {
    let mut w = world();
    w.panes.clear();
    w.agents = vec![
        ("a".into(), Some("S".into())),
        ("b".into(), Some("S".into())),
    ];
    let c = resolve(&w, &env(Some("w6:p48"), Some("S"))).unwrap();
    assert_eq!(c.pane_id.as_deref(), Some("w6:p48"));
    assert_eq!((c.tab_id, c.tab_label), (None, None));
    assert_eq!(c.session_id.as_deref(), Some("S")); // the environment's
    // herdr unreachable: the same
    w.agents.clear();
    let c = resolve(&w, &env(Some("w6:p48"), None)).unwrap();
    assert_eq!((c.pane_id.as_deref(), c.session_id), (Some("w6:p48"), None));
}

#[test]
fn the_session_falls_back_to_the_environment_and_follows_continuations() {
    let mut w = world();
    w.panes.insert("w1:p1".into(), pane("w1:t1", None)); // herdr knows the pane, not the session
    w.continued.insert("claude-sess".into(), "live-sess".into());
    let c = resolve(&w, &env(Some("w1:p1"), Some("claude-sess"))).unwrap();
    assert_eq!(c.session_id.as_deref(), Some("live-sess"));
    let c = resolve(&w, &env(Some("w1:p1"), Some(""))).unwrap();
    assert_eq!(c.session_id, None);
}

#[test]
fn herdrs_tab_wins_over_the_environments() {
    let mut e = env(Some("w1:p1"), None);
    e.tab_id = Some("ENV-TAB".into());
    assert_eq!(
        resolve(&world(), &e).unwrap().tab_id.as_deref(),
        Some("w1:t1")
    );
    let mut w = world();
    w.nested = true;
    assert_eq!(resolve(&w, &e).unwrap().tab_id.as_deref(), Some("ENV-TAB"));
}

#[test]
fn reading_the_marks_fails_as_the_store_does() {
    let mut w = world();
    w.marks_fail = true;
    assert!(matches!(
        resolve(&w, &env(Some("w1:p1"), None)),
        Err(Failure::NotReady)
    ));
    // no label, no store read
    w.tabs.clear();
    assert!(resolve(&w, &env(Some("w1:p1"), None)).is_ok());
}

/// review-#53 L3: what a session runs is judged from the pane asked about. Here OLD
/// lives on as S only in pane w6:p57 (an in-process continuation there); from anywhere
/// else OLD is OLD (another pane may run OLD resumed).
#[test]
fn live_sessions_are_judged_from_the_pane_that_runs_them() {
    let mut w = world();
    w.panes.clear();
    w.panes.insert("w6:p57".into(), pane("w6:t2", Some("OLD")));
    w.in_pane
        .insert(("OLD".into(), "w6:p57".into()), "S".into());
    w.agents = vec![("w6:p57".into(), Some("OLD".into()))];
    // the moved pane is found: its agent's OLD, seen from w6:p57, is S
    assert_eq!(pane_running(&w, Some("S")).as_deref(), Some("w6:p57"));
    let c = resolve(&w, &env(Some("w6:p48"), Some("S"))).unwrap();
    assert_eq!(c.pane_id.as_deref(), Some("w6:p57"));
    // and herdr's OLD files under S, judged from the pane it runs in, not the dead one
    assert_eq!(c.session_id.as_deref(), Some("S"));
}
