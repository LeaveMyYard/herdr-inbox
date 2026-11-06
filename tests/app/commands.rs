//! `tests/fixtures/app-commands/commands.json`, every case against the real verbs, the real
//! store and a recording of what they start. A case `needs: "B3"` runs with core delivery on.

use std::fs;
use std::time::Duration;

use herdr_inbox::app::feed::snapshot;
use herdr_inbox::app::verbs::route::{Effects, Mark};
use herdr_inbox::app::verbs::{Ctx, run};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::world::{World, fixture, wall};

/// What a verb started, in order.
#[derive(Debug, Default)]
pub struct Started {
    /// What `server_gone` says.
    pub gone: bool,
    /// Workers ensured.
    pub workers: Vec<Option<String>>,
    /// Senders: arguments and socket.
    pub senders: Vec<(Vec<String>, Option<String>)>,
    /// Every step's name.
    pub order: Vec<&'static str>,
}

impl Effects for Started {
    fn ensure_worker(&mut self, server: Option<&str>) {
        self.workers.push(server.map(str::to_string));
        self.order.push("worker");
    }
    fn server_gone(&mut self, _: Option<&str>) -> bool {
        self.order.push("ask");
        self.gone
    }
    fn core_gone(&mut self) -> bool {
        self.order.push("ask");
        self.gone
    }
    fn start_sender(&mut self, questions: &[i64], force: bool, socket: Option<&str>) {
        let mut args = vec!["deliver".to_string()];
        args.extend(questions.iter().map(i64::to_string));
        if force {
            args.push("--force".to_string());
        }
        self.senders.push((args, socket.map(str::to_string)));
        self.order.push("sender");
    }
    fn refresh_marks(&mut self, _: &[Mark]) {}
}

/// `argv` of a case, as a mod passes it: a row with no token has nothing to echo.
fn words(case: &Value) -> Vec<String> {
    let argv = case["argv"].as_array().unwrap();
    argv[2..]
        .iter()
        .map(|w| w.as_str().unwrap_or_default().to_string())
        .collect()
}

fn feed_of(name: &str) -> Value {
    fixture(&format!("app-feed/{name}"))
}

fn exec(db: &World, sql: &str, id: i64) {
    db.store.db().execute(sql, [id]).unwrap();
}

/// A claim made just now, by a process alive.
pub fn live_claim() -> String {
    format!("sending:{}:abcd1234", herdr_inbox::clock::now())
}

/// What `about` says changed after the feed was read.
fn setup(name: &str, w: &World) {
    match name {
        "stale-answered" => exec(
            w,
            "UPDATE questions SET status = 'answered', answer = 'x' WHERE id = ?",
            12,
        ),
        "stale-dismissed" => exec(
            w,
            "UPDATE questions SET status = 'dismissed' WHERE id = ?",
            12,
        ),
        "stale-withdrawn" => exec(
            w,
            "UPDATE questions SET status = 'withdrawn' WHERE id = ?",
            12,
        ),
        "stale-removed" => {
            exec(w, "DELETE FROM questions WHERE id = ?", 12);
            exec(w, "DELETE FROM question_ext WHERE id = ?", 12);
        }
        "stale-delivered" => exec(
            w,
            "UPDATE questions SET delivered_at = '2026-01-02T11:00:00+00:00' WHERE id = ?",
            8,
        ),
        "stale-fetched" => exec(
            w,
            "UPDATE questions SET acknowledged_at = '2026-01-02T11:00:00+00:00' WHERE id = ?",
            8,
        ),
        "stale-hidden" => exec(
            w,
            "UPDATE question_ext SET hidden_at = '2026-01-02T11:00:00+00:00' WHERE id = ?",
            8,
        ),
        "stale-acked" => exec(w, "UPDATE alerts SET status = 'acked' WHERE id = ?", 6),
        "stale-cleared" => exec(w, "UPDATE alerts SET status = 'cleared' WHERE id = ?", 6),
        "show-closed" => exec(
            w,
            "UPDATE questions SET status = 'answered', answer = 'x' WHERE id = ?",
            12,
        ),
        "retry-already-sending" => claim(w, 8),
        "stop-showing-during-claim" => claim(w, 30),
        _ => {}
    }
}

fn claim(w: &World, id: i64) {
    w.store
        .db()
        .execute(
            "UPDATE questions SET delivery_error = ? WHERE id = ?",
            params![live_claim(), id],
        )
        .unwrap();
}

/// The row the case acts on, for its stored server and session.
fn stored(case: &Value, w: &World) {
    let key = word_after(case, "--entry").unwrap_or_default();
    let ids: Vec<i64> = match key.split_once(':') {
        Some(("group", g)) => w
            .store
            .group(g.parse().unwrap(), true)
            .unwrap()
            .iter()
            .map(|q| q.id)
            .collect(),
        Some(("failed", q)) => vec![q.parse().unwrap()],
        _ => Vec::new(),
    };
    for id in ids {
        w.store
            .db()
            .execute(
                "UPDATE question_ext SET herdr_server = ? WHERE id = ?",
                params![case["server"].as_str(), id],
            )
            .unwrap();
        if case["session"] == false {
            exec(w, "UPDATE questions SET session_id = NULL WHERE id = ?", id);
        }
    }
}

fn word_after(case: &Value, flag: &str) -> Option<String> {
    let argv = case["argv"].as_array().unwrap();
    let at = argv.iter().position(|w| w == flag)?;
    argv[at + 1].as_str().map(str::to_string)
}

fn case_world(case: &Value) -> World {
    let name = case["name"].as_str().unwrap();
    let mut feed = feed_of(case["world"].as_str().unwrap());
    if name == "show" {
        let row = &case["stdout"]["row"];
        let entries = feed["entries"].as_array_mut().unwrap();
        let shown = entries.iter_mut().find(|e| e["key"] == row["key"]).unwrap();
        shown["questions"][0]["question"] = row["questions"][0]["question"].clone();
    }
    let w = World::of(&feed);
    if name == "show-left-out" {
        w.row(&case["stdout"]["row"]);
    }
    stored(case, &w);
    setup(name, &w);
    w
}

fn run_case(case: &Value, w: &World, fx: &mut Started) -> herdr_inbox::app::verbs::reply::Reply {
    let mut ctx = Ctx {
        db: &w.db,
        db_wait: Duration::from_millis(100),
        core_delivery: case["needs"] == "B3",
        fx,
    };
    run(&words(case), &mut ctx)
}

fn started(fx: &Started) -> Value {
    if fx.workers.is_empty() && fx.senders.is_empty() {
        return Value::Null;
    }
    assert_eq!(fx.workers.len(), 1, "{fx:?}");
    let sender = fx.senders.first();
    json!({
        "ensure_worker": fx.workers[0],
        "sender": sender.map(|s| s.0.clone()),
        "sender_socket": sender.and_then(|s| s.1.clone()),
    })
}

fn expect(case: &Value, reply: &herdr_inbox::app::verbs::reply::Reply, fx: &Started) {
    let name = &case["name"];
    assert_eq!(
        i64::from(reply.exit),
        case["exit"].as_i64().unwrap(),
        "{name}: {reply:?}"
    );
    if case["stdout"].is_null() {
        assert_eq!(reply.stdout, "", "{name}");
    } else {
        let got: Value = serde_json::from_str(&reply.stdout).unwrap();
        assert_eq!(got, case["stdout"], "{name}");
    }
    assert_eq!(started(fx), case["effects"], "{name}");
    if let Some(first) = fx.order.first() {
        assert_eq!(*first, "worker", "{name}: the worker goes first");
    }
}

/// The feed as the writer would publish it now, at the world's clock.
fn now_feed(w: &World, world: &Value) -> Value {
    snapshot::read(&w.store, wall(world)).unwrap()
}

/// The cases that need a world of their own.
const OWN_WORLD: [&str; 4] = ["busy", "not-migrated", "error", "retry-replay"];

#[test]
fn every_case_does_what_the_fixture_says() {
    let doc = fixture("app-commands/commands.json");
    for case in doc["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if OWN_WORLD.contains(&name) {
            continue;
        }
        let w = case_world(case);
        let world = feed_of(case["world"].as_str().unwrap());
        let before = now_feed(&w, &world);
        let mut fx = Started {
            gone: case["host"] == "gone",
            ..Started::default()
        };
        let reply = run_case(case, &w, &mut fx);
        expect(case, &reply, &fx);
        let after = now_feed(&w, &world);
        if reply.exit != 0 {
            assert_eq!(after, before, "{name}: a refusal changes nothing");
        }
        if reply.exit == 0 && case["stdout"]["verb"] != "show" {
            let token = &case["stdout"]["token"];
            let row = after["entries"].as_array().unwrap().iter().find(|e| {
                Some(e["key"].as_str().unwrap().to_string()) == word_after(case, "--entry")
            });
            let published = row.map_or(Value::Null, |r| r["token"].clone());
            assert_eq!(
                &published, token,
                "{name}: the result's token is the writer's"
            );
        }
    }
}

fn case(name: &str) -> Value {
    fixture("app-commands/commands.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()
        .clone()
}

#[test]
fn a_replayed_retry_is_refused_and_cannot_force_a_second_send() {
    let replay = case("retry-replay");
    let first = case("retry-tui");
    let w = case_world(&first);
    let mut fx = Started::default();
    expect(&first, &run_case(&first, &w, &mut fx), &fx);
    let mut again = Started::default();
    expect(&replay, &run_case(&replay, &w, &mut again), &again);
    assert!(again.senders.is_empty());
    assert_eq!(w.store.retry_count(8).unwrap(), 1);
}

#[test]
fn a_write_lock_held_for_the_whole_wait_is_busy() {
    let c = case("busy");
    let w = case_world(&c);
    let other = Connection::open(&w.db).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut fx = Started::default();
    expect(&c, &run_case(&c, &w, &mut fx), &fx);
    other.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn a_database_not_at_this_revision_starts_the_worker_that_migrates_it() {
    let c = case("not-migrated");
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TABLE questions (id INTEGER)")
        .unwrap();
    let mut fx = Started::default();
    let mut ctx = Ctx {
        db: &db,
        db_wait: Duration::from_millis(100),
        core_delivery: false,
        fx: &mut fx,
    };
    let reply = run(&words(&c), &mut ctx);
    assert_eq!(reply.exit, 1);
    let got: Value = serde_json::from_str(&reply.stdout).unwrap();
    assert_eq!(got, c["stdout"]);
    assert_eq!(fx.workers, [None]);
    assert!(fx.senders.is_empty());
}

#[test]
fn a_database_file_that_cannot_be_read_is_an_error_with_no_path_in_it() {
    let c = case("error");
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    fs::create_dir(&db).unwrap();
    let mut fx = Started::default();
    let mut ctx = Ctx {
        db: &db,
        db_wait: Duration::from_millis(100),
        core_delivery: false,
        fx: &mut fx,
    };
    let reply = run(&words(&c), &mut ctx);
    assert_eq!(reply.exit, 1);
    let got: Value = serde_json::from_str(&reply.stdout).unwrap();
    assert_eq!(got, c["stdout"]);
}
