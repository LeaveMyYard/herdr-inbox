use serde_json::json;

use super::checks::items;
use crate::mcp::args::{Ask, Item};

#[test]
fn an_empty_question_or_list_counts_as_none() {
    let refused =
        json!({"status": "error", "message": "Pass either `question` or `questions` (not both)."});
    let empty = Ask {
        question: Some(String::new()),
        questions: Some(Vec::new()),
        ..Ask::default()
    };
    assert_eq!(items(&empty), Err(refused.clone()));
    let one_and_empty_list = Ask {
        question: Some("q?".into()),
        questions: Some(Vec::new()),
        ..Ask::default()
    };
    assert_eq!(items(&one_and_empty_list).unwrap()[0].question, "q?");
}

#[test]
fn one_question_carries_the_calls_options_header_and_multi_select() {
    let a = Ask {
        question: Some("q?".into()),
        options: Some(vec![json!("a")]),
        header: Some("H".into()),
        multi_select: true,
        ..Ask::default()
    };
    assert_eq!(
        items(&a),
        Ok(vec![Item {
            question: "q?".into(),
            header: Some("H".into()),
            options: Some(vec![json!("a")]),
            multi_select: true
        }])
    );
}

#[test]
fn the_option_limit_applies_to_every_item() {
    let item = |n: usize| Item {
        question: "q".into(),
        header: None,
        options: Some((0..n).map(|i| json!(i.to_string())).collect()),
        multi_select: false,
    };
    let a = Ask {
        questions: Some(vec![item(9), item(10)]),
        ..Ask::default()
    };
    assert_eq!(
        items(&a),
        Err(
            json!({"status": "error", "message": "At most 9 options; ask fewer or split the question."})
        )
    );
    let ok = Ask {
        questions: Some(vec![item(9); 4]),
        ..Ask::default()
    };
    assert_eq!(items(&ok).unwrap().len(), 4);
}
