//! `own_answer_qids` and `own_answer_succeeded` against Python's, over the
//! captured commands and outputs (tests/fixtures/evidence.json).

use serde_json::Value;

use super::*;

const FIXTURE: &str = include_str!("../../../../tests/fixtures/evidence.json");

#[test]
fn every_command_answers_what_python_says_it_answers() {
    let data: Value = serde_json::from_str(FIXTURE).unwrap();
    for case in data["commands"].as_array().unwrap() {
        let command = case["command"].as_str().unwrap();
        // an id past 64 bits never names a question: Python keeps it, Rust drops it
        let want: BTreeSet<i64> = case["qids"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_i64)
            .collect();
        assert_eq!(own_answer_qids(command), want, "{command:?}");
    }
}

#[test]
fn every_output_reads_as_pythons() {
    let data: Value = serde_json::from_str(FIXTURE).unwrap();
    for case in data["outputs"].as_array().unwrap() {
        let output = case["output"].as_str().unwrap();
        let qid = case["qid"].as_i64().unwrap();
        assert_eq!(
            own_answer_succeeded(output, qid),
            case["succeeded"].as_bool().unwrap(),
            "{output:?}"
        );
    }
}
