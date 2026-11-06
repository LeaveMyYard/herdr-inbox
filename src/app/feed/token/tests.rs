use serde_json::{Value, json};

use super::token;

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/app-feed/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every token in the fixtures is the document's formula, computed in Python.
#[test]
fn every_published_token_is_the_documents_hash() {
    for name in ["every-kind.json", "stale-token.json", "partial.json"] {
        for e in fixture(name)["entries"].as_array().unwrap() {
            assert_eq!(token(e), e["token"].as_str().map(str::to_string), "{e}");
        }
    }
}

/// A newer writer's fields stay out of the hash: the list is frozen.
#[test]
fn fields_past_the_frozen_list_do_not_move_a_token() {
    let doc = fixture("every-kind.json");
    let mut row = doc["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"] == "group:14")
        .unwrap()
        .clone();
    let before = token(&row);
    row["later"] = json!(1);
    row["asker"]["later"] = json!(2);
    row["questions"][0]["later"] = json!(3);
    row["questions"][0]["options"][0]["later"] = json!(4);
    row["questions"][0]["draft"]["later"] = json!(5);
    row["questions"][0]["cut"] = json!(true);
    row["actions"] = json!([]);
    assert_eq!(token(&row), before);
}

#[test]
fn each_counted_field_moves_a_token() {
    let doc = fixture("every-kind.json");
    let rows = doc["entries"].as_array().unwrap();
    let group = rows.iter().find(|e| e["key"] == "group:14").unwrap();
    let before = token(group);
    let moves: [fn(&mut Value); 6] = [
        |r| r["title"] = json!("Other Title"),
        |r| r["asker"]["terminal"] = json!("t9"),
        |r| r["questions"][0]["question"] = json!("Other Text"),
        |r| r["questions"][0]["options"][0]["label"] = json!("Other"),
        |r| r["questions"][0]["draft"]["text"] = json!("Other"),
        |r| {
            r["questions"].as_array_mut().unwrap().pop();
        },
    ];
    for change in moves {
        let mut row = group.clone();
        change(&mut row);
        assert_ne!(token(&row), before, "{row}");
    }
    let failed = rows.iter().find(|e| e["key"] == "failed:8").unwrap();
    let mut row = failed.clone();
    row["retries"] = json!(1);
    assert_ne!(token(&row), token(failed));
    row = failed.clone();
    row["delivery"]["state"] = json!("failed");
    assert_eq!(token(&row), token(failed), "delivery moves all the time");
}

#[test]
fn a_kind_without_verbs_has_no_token() {
    assert_eq!(token(&json!({"kind": "remote_group"})), None);
    assert_eq!(token(&json!({})), None);
}
