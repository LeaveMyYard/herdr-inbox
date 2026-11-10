//! `tests/fixtures/app-feed/*.json`, reproduced from a database.

use herdr_inbox::app::feed::snapshot;
use herdr_inbox::app::feed::text::LONG;
use serde_json::{Value, json};

use crate::world::{World, fixture, wall};

fn reproduces(name: &str) {
    let want = fixture(&format!("app-feed/{name}"));
    let world = World::of(&want);
    assert_same(&snapshot::read(&world.store, wall(&want)).unwrap(), &want);
}

fn assert_same(got: &Value, want: &Value) {
    let (a, b) = (
        got["entries"].as_array().unwrap(),
        want["entries"].as_array().unwrap(),
    );
    for (x, y) in a.iter().zip(b) {
        assert_eq!(x, y, "row {}", y["key"]);
    }
    assert_eq!(got, want);
}

#[test]
fn every_kind_is_the_feed_of_its_world() {
    reproduces("every-kind.json");
}

#[test]
fn the_world_after_a_withdrawal_gives_the_stale_token_feed() {
    reproduces("stale-token.json");
}

#[test]
fn nothing_open_is_an_empty_feed_with_its_marks() {
    reproduces("empty.json");
}

/// A row no feed can hold: over 1 MiB in a field that is never cut.
fn huge() -> String {
    "x".repeat(1024 * 1024 + 1)
}

/// Group 21, the two questions the fixtures leave out.
fn left_out(world: &World) {
    let row = fixture("app-commands/commands.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "show-left-out")
        .unwrap()["stdout"]["row"]
        .clone();
    let mut row = row;
    row["questions"][0]["asked_by"] = json!(huge());
    world.row(&row);
}

#[test]
fn a_group_too_big_for_any_feed_is_left_out_and_counted() {
    let want = fixture("app-feed/omitted-only.json");
    let world = World::of(&want);
    left_out(&world);
    assert_same(&snapshot::read(&world.store, wall(&want)).unwrap(), &want);
}

#[test]
fn partial_cuts_what_is_long_and_counts_what_is_left_out() {
    let want = fixture("app-feed/partial.json");
    // the world holds the whole text; the fixture shows it cut
    let mut world_rows = want.clone();
    let shown = fixture("app-commands/commands.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "show")
        .unwrap()["stdout"]["row"]["questions"][0]["question"]
        .clone();
    assert!(shown.as_str().unwrap().len() > LONG);
    let entries = world_rows["entries"].as_array_mut().unwrap();
    let group = entries.iter_mut().find(|e| e["key"] == "group:14").unwrap();
    group["questions"][0]["question"] = shown;
    let world = World::of(&world_rows);
    left_out(&world);
    // one `failed` row and one remote row too big to fit, and one the mirror cannot name
    let mut failed = fixture("app-feed/every-kind.json")["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"] == "failed:13")
        .unwrap()
        .clone();
    failed["asked_by"] = json!(huge());
    world.row(&failed);
    let mut remote = fixture("app-feed/every-kind.json")["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "remote_alert")
        .unwrap()
        .clone();
    remote["tab_label"] = json!(huge());
    world.row(&remote);
    world.remote_question(
        "machine-alpha",
        "db-alpha",
        99,
        "open",
        &json!({"status": "open"}),
    );
    assert_same(&snapshot::read(&world.store, wall(&want)).unwrap(), &want);
}
