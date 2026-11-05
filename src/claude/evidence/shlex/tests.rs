//! The splitter against Python's shlex, over the captured commands
//! (tests/fixtures/evidence.json, tools/capture_evidence.py).

use serde_json::Value;

use super::*;

pub(crate) const FIXTURE: &str = include_str!("../../../../tests/fixtures/evidence.json");

#[test]
fn every_command_splits_as_pythons_shlex_splits_it() {
    let data: Value = serde_json::from_str(FIXTURE).unwrap();
    for case in data["commands"].as_array().unwrap() {
        let command = case["command"].as_str().unwrap();
        let got = split(command);
        match &case["tokens"] {
            Value::Array(want) => {
                let want: Vec<&str> = want.iter().map(|t| t.as_str().unwrap()).collect();
                assert_eq!(
                    got.as_deref().ok(),
                    Some(
                        want.iter()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                            .as_slice()
                    ),
                    "{command:?}"
                );
            }
            _ => assert_eq!(got, Err(Unbalanced), "{command:?}"),
        }
    }
}
