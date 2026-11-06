use serde_json::{Value, json};

use super::{Unfit, normalise};
use crate::store::question::{QOption, Question};

fn question(labels: &[&str], multi: bool) -> Question {
    Question {
        id: 7,
        multi_select: multi,
        options: Some(
            labels
                .iter()
                .map(|l| QOption {
                    label: (*l).to_string(),
                    ..QOption::default()
                })
                .collect(),
        ),
        ..Question::default()
    }
}

fn detail(choices: &[&str], notes: Value, text: Value) -> Value {
    json!({"choices": choices, "notes": notes, "text": text})
}

#[test]
fn choices_come_out_in_option_order() {
    let q = question(&["A", "B", "C"], true);
    let d = normalise(&q, &detail(&["C", "A"], json!({}), json!(null))).unwrap();
    assert_eq!(d, Some(detail(&["A", "C"], json!({}), json!(null))));
}

#[test]
fn notes_stay_only_on_picked_labels_stripped_and_text_is_stripped() {
    let q = question(&["A", "B"], true);
    let got = normalise(
        &q,
        &detail(&["A"], json!({"A": "  x  ", "B": "y"}), json!("  z ")),
    )
    .unwrap();
    assert_eq!(got, Some(detail(&["A"], json!({"A": "x"}), json!("z"))));
    let blank = normalise(&q, &detail(&["A"], json!({"A": "   "}), json!("   "))).unwrap();
    assert_eq!(blank, Some(detail(&["A"], json!({}), json!(null))));
}

#[test]
fn nothing_picked_and_no_text_is_nothing() {
    let q = question(&["A"], false);
    assert_eq!(normalise(&q, &detail(&[], json!({}), json!(" "))), Ok(None));
    assert_eq!(
        normalise(&q, &detail(&[], json!({"A": "x"}), json!(null))),
        Ok(None)
    );
}

#[test]
fn a_choice_must_be_a_label_once_and_one_without_multi_select() {
    let q = question(&["A", "B"], false);
    let refused = |d: Value| normalise(&q, &d).unwrap_err();
    assert_eq!(
        refused(detail(&["Z"], json!({}), json!(null))),
        Unfit("Q7 has no such option".to_string())
    );
    assert_eq!(
        refused(detail(&["A", "B"], json!({}), json!(null))),
        Unfit("Q7 takes one choice".to_string())
    );
    let multi = question(&["A", "B"], true);
    assert_eq!(
        normalise(&multi, &detail(&["A", "A"], json!({}), json!(null))),
        Err(Unfit("Q7 has a choice twice".to_string()))
    );
    let free = Question {
        id: 7,
        ..Question::default()
    };
    assert!(normalise(&free, &detail(&["A"], json!({}), json!(null))).is_err());
}

/// `normalize_options` does not dedupe: a label then means the first such option.
#[test]
fn two_options_with_one_label_read_as_one() {
    let q = question(&["A", "B", "A"], true);
    let d = normalise(&q, &detail(&["B", "A"], json!({}), json!(null))).unwrap();
    assert_eq!(d, Some(detail(&["A", "B"], json!({}), json!(null))));
}
