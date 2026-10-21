//! The messages of one pane's answers against Python's
//! (tests/fixtures/delivery-messages.json, `tools/capture_goldens.py --only
//! delivery`): the cap, the long-answer reference, the batches.

use herdr_agent_kit::internal::inbox_contract::delivery::Answered;
use herdr_inbox::delivery::messages::{Message, delivery_messages};
use serde_json::Value;

const GOLDEN: &str = include_str!("fixtures/delivery-messages.json");

#[test]
fn every_set_of_answers_splits_as_python_splits_it() {
    let doc: Value = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(doc["max_arg"], herdr_agent_kit::herdr_api::MAX_ARG);
    let rows = doc["rows"].as_array().unwrap();
    assert!(rows.len() >= 13);
    for row in rows {
        let answers: Vec<Answered<'_>> = row["answers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| Answered {
                id: a["id"].as_u64().unwrap(),
                question: a["question"].as_str().unwrap(),
                answer: a["answer"].as_str().unwrap(),
            })
            .collect();
        let want: Vec<Message> = row["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| Message {
                text: m["text"].as_str().unwrap().into(),
                ids: m["ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| i.as_u64().unwrap())
                    .collect(),
                full: m["ack"].as_bool().unwrap(),
            })
            .collect();
        assert_eq!(delivery_messages(&answers), want, "{}", row["name"]);
    }
}
