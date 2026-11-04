//! What the Python scenarios cannot show: a database that fails mid-pass,
//! when each outcome is written, and the one wait per typed message however
//! many answers it carries. Python leaves `deliver_batch` at a failing
//! statement, having written each earlier outcome as it was decided; so
//! does the Rust pass (#100 review H1, M1).

use std::time::{Duration, Instant};

use herdr_inbox::clock::{now, now_minus};
use herdr_inbox::delivery::run::BUSY_GRACE;
use inbox_core::deliver::{Mode, NOT_VERIFIED};
use serde_json::{Value, json};

use crate::common::open;
use crate::run_world::{Ran, World, rows};

/// A long answer: its reference is one message, the short answer the next.
fn long() -> String {
    "x".repeat(5000)
}

fn fails(ran: &Ran, why: &str) {
    let err = ran.result.as_ref().unwrap_err().to_string();
    assert!(err.contains(why), "{err}");
}

/// A trigger that aborts `event` (e.g. `UPDATE OF delivery_error`) on
/// answer `id` when `when` holds of `NEW`.
fn abort(world: &World, event: &str, id: i64, when: &str) {
    open(&world.db)
        .execute_batch(&format!(
            "CREATE TRIGGER fail_{id} BEFORE {event} ON questions WHEN NEW.id = {id} AND {when} \
             BEGIN SELECT RAISE(ABORT, 'disk on fire'); END"
        ))
        .unwrap();
}

/// Every read of the sessions table fails from now on.
fn lose_sessions(store: &herdr_inbox::store::Store) {
    store
        .db()
        .execute_batch("ALTER TABLE sessions RENAME TO sessions_gone")
        .unwrap();
}

#[test]
fn a_database_that_fails_mid_pass_types_nothing_and_says_so() {
    let world = World::new(&["yes"]);
    // the claim (an UPDATE of delivery_error) fails
    abort(&world, "UPDATE OF delivery_error", world.ids[0], "1");
    let ran = world.pass(|_| {});
    fails(&ran, "disk on fire");
    assert_eq!(ran.prompts(), 0);
}

/// #100 review H1, the reviewer's case: the first claim is taken, the
/// second fails. Python raises in the claims comprehension and types
/// nothing; a pass that typed the first answer could never record it.
#[test]
fn a_claim_that_fails_after_another_was_taken_types_nothing() {
    let world = World::new(&["yes", "eu", "squash"]);
    let (q1, q2, q3) = (world.ids[0], world.ids[1], world.ids[2]);
    abort(
        &world,
        "UPDATE OF delivery_error",
        q2,
        "NEW.delivery_error LIKE 'sending:%'",
    );
    let ran = world.pass(|_| {});
    fails(&ran, "disk on fire");
    assert_eq!(ran.prompts(), 0);
    // the first keeps its claim until it goes stale, as Python leaves it;
    // the third is never claimed
    assert_eq!(
        rows(&world.db, &[q1, q2, q3]),
        [
            json!([false, "<claim>"]),
            json!([false, null]),
            json!([false, null])
        ]
    );
}

/// A failed move halts the pass: the next answer is not moved either.
#[test]
fn a_move_that_fails_moves_nothing_more() {
    let mut world = World::new(&["yes", "eu"]);
    world.elsewhere = Some("w1:p2");
    abort(&world, "UPDATE OF pane_id", world.ids[0], "1");
    let ran = world.pass(|_| {});
    fails(&ran, "disk on fire");
    let conn = open(&world.db);
    let panes: Vec<String> = world
        .ids
        .iter()
        .map(|id| {
            conn.query_row("SELECT pane_id FROM questions WHERE id = ?", [id], |r| {
                r.get(0)
            })
            .unwrap()
        })
        .collect();
    assert_eq!(panes, ["w1:p1", "w1:p1"]);
    assert_eq!(
        rows(&world.db, &world.ids[1..]),
        [json!([false, "<claim>"])]
    );
}

/// #100 review H1: a lookup that fails before typing (here the watcher
/// check and the transcript lookup, as a failing `landed_across` would)
/// answers "nothing there", which reads as "not landed yet": nothing may be
/// typed on it.
#[test]
fn a_lookup_that_fails_before_typing_types_nothing() {
    let world = World::new(&["yes"]);
    // an asker without a session: no family lookup before the landing check
    open(&world.db)
        .execute("UPDATE questions SET session_id = NULL", [])
        .unwrap();
    let ran = world.pass(lose_sessions);
    fails(&ran, "sessions");
    assert_eq!(ran.prompts(), 0);
    assert_eq!(rows(&world.db, &world.ids), [json!([false, "<claim>"])]);
}

/// The outcome a failed lookup implies is never written: a family that
/// cannot be read is not "conversation changed".
#[test]
fn a_failed_lookup_writes_no_outcome() {
    let world = World::new(&["yes"]);
    let ran = world.pass(lose_sessions);
    fails(&ran, "sessions");
    assert_eq!(ran.prompts(), 0);
    assert_eq!(rows(&world.db, &world.ids), [json!([false, "<claim>"])]);
}

/// #100 review M1: Python writes a message's outcome before it types the
/// next one; a later message's wait never holds an earlier outcome back.
#[test]
fn each_messages_outcome_is_written_before_the_next_is_typed() {
    let world = World::new(&[&long(), "eu"]);
    let ran = world.pass(|_| {});
    ran.result.as_ref().unwrap();
    assert_eq!(ran.prompts(), 2);
    let q1 = json!([false, NOT_VERIFIED]); // typed while idle, never seen
    assert_eq!(
        ran.at_prompt[0],
        [json!([false, "<claim>"]), json!([false, "<claim>"])]
    );
    assert_eq!(ran.at_prompt[1], [q1.clone(), json!([false, "<claim>"])]);
    assert_eq!(rows(&world.db, &world.ids), [q1.clone(), q1]);
}

/// #100 review M1: a write that fails halts the pass before the next message.
#[test]
fn a_failed_write_types_no_further_message() {
    let world = World::new(&[&long(), "eu"]);
    abort(&world, "UPDATE OF delivery_attempts", world.ids[0], "1");
    let ran = world.pass(|_| {});
    fails(&ran, "disk on fire");
    assert_eq!(ran.prompts(), 1);
    assert_eq!(
        rows(&world.db, &world.ids[1..]),
        [json!([false, "<claim>"])]
    );
}

/// #100 review M1: a verify-later job is handed over right after its
/// prompt, before the next message is typed; a long answer's reference is
/// not the full answer (`ack`, #100 review M2).
#[test]
fn verify_later_jobs_are_handed_over_before_the_next_message() {
    let mut world = World::new(&[&long(), "eu"]);
    world.mode = Mode {
        verify_later: true,
        ..Mode::default()
    };
    let ran = world.pass(|_| {});
    ran.result.as_ref().unwrap();
    let got: Vec<(Value, usize)> = ran
        .jobs
        .iter()
        .map(|(job, typed)| (job["ack"].clone(), *typed))
        .collect();
    assert_eq!(got, [(json!(false), 1), (json!(true), 2)]);
}

/// Runs `step` until the clock second did not change across it.
fn within_one_second<T>(mut step: impl FnMut() -> T) -> T {
    for _ in 0..5 {
        let before = now();
        let got = step();
        if now() == before {
            return got;
        }
    }
    panic!("the clock second changed on every try");
}

/// #100 review L1: pending exactly BUSY_GRACE ago is past the grace
/// (Python `_busy_for(...) < BUSY_GRACE` is false): typed.
#[test]
fn the_busy_grace_ends_at_exactly_busy_grace() {
    let typed = within_one_second(|| {
        let mut world = World::new(&["yes"]);
        world.status = "working";
        world.wait = Duration::from_millis(50);
        open(&world.db)
            .execute(
                "INSERT INTO question_ext (id, group_id, pending_since) VALUES (?, ?, ?)",
                rusqlite::params![world.ids[0], world.ids[0], now_minus(BUSY_GRACE)],
            )
            .unwrap();
        world.pass(|_| {}).prompts()
    });
    assert_eq!(typed, 1);
}

#[test]
fn a_message_is_waited_for_once_whatever_it_carries() {
    let mut world = World::new(&["yes", "eu", "squash"]);
    world.wait = Duration::from_millis(800);
    let started = Instant::now();
    let ran = world.pass(|_| {});
    ran.result.as_ref().unwrap();
    let took = started.elapsed();
    assert_eq!(ran.prompts(), 1); // the three in one message
    assert!(
        took < Duration::from_millis(1600),
        "waited per answer: {took:?}"
    );
}

/// #100 review round 2 M1: building a verify-later job reads its plans; when
/// that fails after the prompt, no job goes out (Python raises before
/// `verify_later`): a job without its plans would record the answer as never
/// seen under the claim.
#[test]
fn a_job_whose_plans_fail_is_never_handed_over() {
    let mut world = World::new(&["yes"]);
    world.mode = Mode {
        verify_later: true,
        ..Mode::default()
    };
    world.lose_sessions_at_prompt = true;
    let ran = world.pass(|_| {});
    fails(&ran, "sessions");
    assert_eq!(ran.prompts(), 1);
    assert!(ran.jobs.is_empty(), "{:?}", ran.jobs);
    assert_eq!(rows(&world.db, &world.ids), [json!([false, "<claim>"])]);
}

/// #100 review round 2 L1: a halted pass tells the operator nothing either
/// (Python never reaches the input box): no "cannot read" notice, no "not sent".
#[test]
fn a_halted_pass_shows_no_notice() {
    let mut world = World::new(&["yes", "eu"]);
    world.mode.force = true;
    world.screen = "plain output";
    abort(
        &world,
        "UPDATE OF delivery_error",
        world.ids[1],
        "NEW.delivery_error LIKE 'sending:%'",
    );
    let ran = world.pass(|_| {});
    fails(&ran, "disk on fire");
    assert_eq!(ran.prompts(), 0);
    assert!(ran.notes.is_empty(), "{:?}", ran.notes);
}
