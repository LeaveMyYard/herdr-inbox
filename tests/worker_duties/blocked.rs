//! The blocked-agent scan tick by tick (Python tests/test_alerts.py, its
//! `worker.tick` scenarios): when a prompt becomes an alert, what clears it,
//! what a flicker, a dismissal, another server's row and a restart change.
//! BLOCKED_AFTER is a constant here: a test moves `blocked_panes.since` back
//! instead of Python's patched 0, and the clock it hands the scan stands for
//! CLEAR_AFTER (10 s).

use std::time::Duration;

use herdr_inbox::clock::now_minus;
use herdr_inbox::store::alerts::BlockedScan;
use herdr_inbox::worker::duties::Round;
use herdr_inbox::worker::duties::alerts::scan_blocked;
use herdr_inbox::worker::duties::tick::tick;
use serde_json::{Value, json};

use crate::World;

fn agent(status: &str) -> Value {
    json!([{"pane_id": "w1:p1", "tab_id": "w1:t1", "agent": "claude", "agent_status": status,
            "agent_session": {"value": crate::A}}])
}

impl World {
    /// herdr reports w1:p1's agent as `status`, and the scan sees it at `t`.
    fn status_at(&mut self, scan: &mut BlockedScan, status: &str, t: f64) {
        self.fake.set(|k| k.agents = agent(status));
        self.scan(scan, t);
    }

    /// Every prompt on record has been up longer than BLOCKED_AFTER.
    fn past_blocked_after(&self) {
        self.sql(
            "UPDATE blocked_panes SET since = ?",
            [now_minus(Duration::from_secs(20))],
        );
    }

    /// The notifications that say an agent waits.
    fn waiting_notes(&self) -> usize {
        self.notes
            .iter()
            .filter(|(_, b)| b.contains("waiting"))
            .count()
    }
}

#[test]
fn an_agent_stuck_at_a_prompt_becomes_an_alert_and_clears_itself() {
    let mut w = World::new();
    let mut scan = BlockedScan::default();
    w.blocked_alert(&mut scan);
    w.scan(&mut scan, 2.0);
    let open = w.store.open_alerts().unwrap();
    let [a] = open.as_slice() else {
        panic!("{open:?}")
    };
    assert_eq!(
        (a.kind.as_str(), a.title.as_str()),
        ("blocked", "my-tab is waiting for you")
    );
    assert_eq!(w.waiting_notes(), 1);
    assert_eq!(w.store.open_counts().unwrap().get("w1:p1"), Some(&1));

    w.status_at(&mut scan, "working", 3.0);
    assert_eq!(
        w.open_alerts(),
        1,
        "one scan of another status is a flicker"
    );
    w.status_at(&mut scan, "working", 20.0);
    assert_eq!(w.open_alerts(), 0);
    assert_eq!(w.store.alert(a.id).unwrap().unwrap().status, "cleared");
    assert!(w.fake.prompts().is_empty());
    let requests = w.fake.stub.requests.lock().unwrap();
    assert!(
        !requests.iter().any(|(m, _)| m.contains("send")),
        "{requests:?}"
    );
}

#[test]
fn a_short_prompt_never_raises_an_alert() {
    let mut w = World::new();
    let mut scan = BlockedScan::default();
    w.status_at(&mut scan, "blocked", 0.0);
    w.status_at(&mut scan, "blocked", 1.0); // a second scan, still inside BLOCKED_AFTER
    assert_eq!(w.open_alerts(), 0);
    assert_eq!(w.waiting_notes(), 0);
}

#[test]
fn a_tick_counts_the_agents_it_watches() {
    let mut w = World::new();
    let mut scan = BlockedScan::default();
    let mut watching = |w: &mut World, agents: Value| {
        w.fake.set(|k| k.agents = agents);
        let mut memory = std::mem::take(&mut w.memory);
        let tock = w.with_round(|round| {
            let mut blocked = |r: &mut Round<'_, '_>| scan_blocked(r, &mut scan, 0.0, &mut |_| {});
            tick(round, &mut memory, &mut blocked).unwrap()
        });
        w.memory = memory;
        tock.watching
    };
    assert_eq!(watching(&mut w, agent("idle")), 1);
    assert_eq!(watching(&mut w, json!([])), 0);
}

#[test]
fn a_dismissed_blocked_alert_holds_until_the_prompt_ends() {
    let mut w = World::new();
    let mut scan = BlockedScan::default();
    w.blocked_alert(&mut scan);
    let id = w.store.open_alerts().unwrap()[0].id;
    w.store.acknowledge_alert(id).unwrap().unwrap(); // the prompt is still up
    w.scan(&mut scan, 2.0);
    w.scan(&mut scan, 3.0);
    assert_eq!(w.open_alerts(), 0);
    w.status_at(&mut scan, "working", 4.0);
    w.status_at(&mut scan, "working", 20.0);
    w.status_at(&mut scan, "blocked", 21.0); // the next prompt
    w.past_blocked_after();
    w.scan(&mut scan, 22.0);
    assert_eq!(w.open_alerts(), 1);
}

#[test]
fn a_flickering_status_neither_clears_nor_renotifies() {
    let mut w = World::new();
    let mut scan = BlockedScan::default();
    w.status_at(&mut scan, "blocked", 0.0);
    w.past_blocked_after();
    let flicker = [
        "blocked", "working", "working", "working", "blocked", "working", "blocked",
    ];
    for (i, status) in flicker.into_iter().enumerate() {
        w.status_at(&mut scan, status, 1.0 + i as f64); // all within CLEAR_AFTER
    }
    let open = w.store.open_alerts().unwrap();
    assert_eq!(open.iter().map(|a| a.id).collect::<Vec<_>>(), [1]);
    assert_eq!(w.waiting_notes(), 1);
}

#[test]
fn an_agent_gone_from_the_list_clears_at_once() {
    let mut w = World::new();
    let mut scan = BlockedScan::default();
    w.blocked_alert(&mut scan);
    assert_eq!(w.open_alerts(), 1);
    w.fake.set(|k| k.agents = json!([]));
    w.scan(&mut scan, 1.5); // well inside CLEAR_AFTER, one scan
    assert_eq!(w.open_alerts(), 0);
}

#[test]
fn another_herdr_servers_alert_for_the_same_pane_id_does_not_suppress() {
    let mut w = World::new();
    for status in ["open", "acked"] {
        w.sql(
            "INSERT INTO alerts (created_at, kind, severity, title, pane_id, herdr_server, status) \
             VALUES ('2026-10-15T00:00:00+00:00', 'blocked', 'warning', 'x', 'w1:p1', '/other.sock', ?)",
            [status],
        );
    }
    let mut scan = BlockedScan::default();
    w.blocked_alert(&mut scan);
    let open = w.store.open_alerts().unwrap();
    let mine: Vec<_> = open
        .iter()
        .filter(|a| a.herdr_server.as_deref() != Some("/other.sock"))
        .collect();
    assert_eq!(mine.len(), 1, "{open:?}");
    assert_eq!(mine[0].pane_id.as_deref(), Some("w1:p1"));
}

#[test]
fn the_blocked_clock_survives_a_worker_restart() {
    let mut w = World::new();
    w.status_at(&mut BlockedScan::default(), "blocked", 0.0);
    w.past_blocked_after();
    w.scan(&mut BlockedScan::default(), 0.0); // a new worker process, a new memory
    assert_eq!(w.open_alerts(), 1);
}
