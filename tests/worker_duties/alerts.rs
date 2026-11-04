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
