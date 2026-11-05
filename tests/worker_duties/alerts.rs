//! The blocked-agent scan through herdr (Python tests/test_alerts.py's
//! worker half): one `agent list`, an alert for an agent at a prompt past
//! BLOCKED_AFTER with its notification and its pane's marks, and how many
//! agents herdr reports (they keep the worker alive).

use herdr_inbox::store::alerts::BlockedScan;
use herdr_inbox::worker::duties::alerts::scan_blocked;
use serde_json::json;

use crate::World;

impl World {
    fn scan(&mut self, scan: &mut BlockedScan, t: f64) -> (usize, Vec<Option<String>>) {
        let mut marked = Vec::new();
        let n = self.with_round(|round| {
            scan_blocked(round, scan, t, &mut |p: Option<&str>| {
                marked.push(p.map(String::from))
            })
        });
        (n, marked)
    }
}

#[test]
fn an_agent_at_a_prompt_gets_one_alert_and_nothing_is_typed() {
    let mut w = World::new();
    w.fake.set(|k| {
        k.agents = json!([{"pane_id": "w1:p1", "tab_id": "w1:t1", "agent": "claude", "agent_status": "blocked",
                           "agent_session": {"value": crate::A}},
                          {"pane_id": "w1:p2", "agent": "claude", "agent_status": "working"}]);
    });
    let mut scan = BlockedScan::default();
    let (n, marked) = w.scan(&mut scan, 0.0);
    assert_eq!(n, 2); // both agents are watched
    assert!(marked.is_empty() && w.notes.is_empty()); // not past BLOCKED_AFTER yet
    w.sql(
        "UPDATE blocked_panes SET since = '2026-01-01T00:00:00+00:00'",
        [],
    );
    let (_, marked) = w.scan(&mut scan, 1.0);
    assert_eq!(marked, [Some("w1:p1".to_string())]);
    assert_eq!(w.notes.len(), 1);
    assert!(w.notes[0].0.ends_with("· my-tab") && w.notes[0].1 == "waiting for you at a prompt");
    w.scan(&mut scan, 2.0);
    assert_eq!(w.notes.len(), 1); // once
    assert!(w.fake.prompts().is_empty());
}

#[test]
fn a_herdr_that_does_not_answer_watches_no_one() {
    let mut w = World::new();
    w.fake.set(|k| k.fail_agent_list = true);
    assert_eq!(w.scan(&mut BlockedScan::default(), 0.0).0, 0);
}

#[test]
fn an_agent_list_that_is_not_a_list_changes_nothing() {
    let mut w = World::new();
    w.fake.set(|k| {
        k.agents = json!([{"pane_id": "w1:p1", "tab_id": "w1:t1", "agent": "claude", "agent_status": "blocked"}]);
    });
    let mut scan = BlockedScan::default();
    w.scan(&mut scan, 0.0);
    w.sql(
        "UPDATE blocked_panes SET since = '2026-01-01T00:00:00+00:00'",
        [],
    );
    w.scan(&mut scan, 1.0);
    let open = |w: &World| -> i64 {
        w.store
            .db()
            .query_row(
                "SELECT COUNT(*) FROM alerts WHERE status = 'open'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(open(&w), 1);
    w.fake.set(|k| k.agents = json!("not a list"));
    for t in [20.0, 40.0] {
        assert_eq!(w.scan(&mut scan, t).0, 0); // Python: a TypeError, `watching = 0`
    }
    assert_eq!(open(&w), 1); // the alert stays: nothing changed
}

impl World {
    /// w1:p1's agent at a prompt, past BLOCKED_AFTER: one open alert.
    fn blocked_alert(&mut self, scan: &mut BlockedScan) {
        self.fake.set(|k| {
            k.agents = json!([{"pane_id": "w1:p1", "tab_id": "w1:t1", "agent": "claude", "agent_status": "blocked"}]);
        });
        self.scan(scan, 0.0);
        self.sql(
            "UPDATE blocked_panes SET since = '2026-01-01T00:00:00+00:00'",
            [],
        );
        self.scan(scan, 1.0);
    }

    fn open_alerts(&self) -> i64 {
        self.store
            .db()
            .query_row(
                "SELECT COUNT(*) FROM alerts WHERE status = 'open'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
}

#[test]
fn a_result_without_agents_is_no_agents() {
    let mut w = World::new();
    let mut scan = BlockedScan::default();
    w.blocked_alert(&mut scan);
    assert_eq!(w.open_alerts(), 1);
    w.fake.set(|k| k.agent_list_result = Some(json!({})));
    assert_eq!(w.scan(&mut scan, 2.0).0, 0);
    assert_eq!(w.open_alerts(), 0); // herdr answered: nobody is at a prompt
}

#[test]
fn a_result_that_is_no_object_changes_nothing() {
    let mut w = World::new();
    let mut scan = BlockedScan::default();
    w.blocked_alert(&mut scan);
    w.fake.set(|k| k.agent_list_result = Some(json!([1])));
    w.scan(&mut scan, 2.0);
    assert_eq!(w.open_alerts(), 1);
}

#[test]
fn a_scan_that_fails_watches_no_one() {
    let mut w = World::new();
    w.fake.set(|k| {
        k.agents = json!([{"pane_id": "w1:p2", "agent": "claude", "agent_status": "blocked"}]);
    });
    w.sql("DROP TABLE blocked_panes", []);
    assert_eq!(w.scan(&mut BlockedScan::default(), 0.0).0, 0);
}
