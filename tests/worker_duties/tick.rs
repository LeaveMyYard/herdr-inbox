//! One tick with a step failing (#132). A step Python runs in its own `try`
//! only logs: the rest of the tick still runs, down to the retry loop. Each
//! test breaks one step with a trigger, checks that the step really failed,
//! and that the tick ended well. Synthetic data.

use std::time::Duration;

use herdr_inbox::clock::now_minus;
use herdr_inbox::worker::duties::tick::{Tock, tick};
use rusqlite::params;
use serde_json::json;

use crate::World;
use crate::failed::{NEW, OLD};

const WATCHED: usize = 7;

impl World {
    /// Fails `what` (`<event> ON <table> [WHEN ...]`).
    fn fault(&self, what: &str) {
        self.store
            .db()
            .execute_batch(&format!(
                "CREATE TRIGGER fault BEFORE {what} BEGIN SELECT RAISE(ABORT, 'injected fault'); END"
            ))
            .unwrap();
    }

    /// A tick, and how often its blocked scan ran.
    fn tick_counting_scans(&mut self) -> (rusqlite::Result<Tock>, usize) {
        let mut memory = std::mem::take(&mut self.memory);
        let mut scans = 0;
        let got = self.with_round(|round| {
            tick(round, &mut memory, &mut |_| {
                scans += 1;
                WATCHED
            })
        });
        self.memory = memory;
        (got, scans)
    }

    /// The tick ran past the failed step: its scan, then the retry loop,
    /// which finds the answer left pending at the agent that is working.
    fn ticks_to_the_end(&mut self) {
        let (got, scans) = self.tick_counting_scans();
        assert_eq!(
            got.unwrap(),
            Tock {
                pending: 1,
                watching: WATCHED
            }
        );
        assert_eq!(scans, 1);
    }
}

fn busy_world() -> World {
    let mut w = World::new();
    w.pending_answer("working");
    w
}

#[test]
fn a_failed_reconcile_leaves_the_tick_running() {
    let mut w = busy_world();
    let qid = w.filed("w1:p3", NEW, "Question Text 301", "yes");
    w.as_stored(qid, "Question Text 302", true);
    w.fault("UPDATE OF delivery_error ON questions WHEN OLD.acknowledged_at IS NOT NULL");
    w.ticks_to_the_end();
    let q = w.store.get(qid).unwrap().unwrap();
    assert!(q.delivery_error.is_some(), "the step failed: {q:?}");
}

#[test]
fn a_failed_revival_leaves_the_tick_running() {
    let mut w = busy_world();
    let qid = w.gone_in_p1();
    w.moved_to_p7();
    w.fault("UPDATE OF pane_id ON questions WHEN NEW.pane_id = 'w1:p7'");
    w.ticks_to_the_end();
    let q = w.store.get(qid).unwrap().unwrap();
    assert_eq!(q.pane_id.as_deref(), Some("w1:p1"), "the step failed");
}

#[test]
fn a_failed_continuation_scan_leaves_the_tick_running() {
    let mut w = busy_world();
    let qid = w.filed("w9:p9", OLD, "Question Text 303", "x");
    w.sql(
        "UPDATE questions SET status = 'open', answer = NULL WHERE id = ?",
        params![qid],
    );
    w.write(
        OLD,
        &[json!({"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW})],
    );
    w.fault("INSERT ON sessions");
    w.ticks_to_the_end();
    let recorded: i64 = w
        .store
        .db()
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(recorded, 0, "the step failed");
}

#[test]
fn a_failed_hook_verification_leaves_the_tick_running() {
    let mut w = busy_world();
    let qid = w.hook_sent(&now_minus(Duration::from_secs(60)));
    w.fault("UPDATE OF hook_sent_at ON question_ext");
    w.ticks_to_the_end();
    assert_eq!(w.store.hook_misses(qid).unwrap(), 0, "the step failed");
}

#[test]
fn a_step_python_runs_outside_a_try_fails_the_tick_after_the_scan() {
    let mut w = busy_world();
    w.sql("ALTER TABLE changes RENAME TO changes_lost", []);
    let (got, scans) = w.tick_counting_scans();
    assert!(got.unwrap_err().to_string().contains("changes"));
    assert_eq!(scans, 1);
}
