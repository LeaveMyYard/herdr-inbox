use serde_json::{Map, Value, json};

use super::lax::{boolean, int};
use super::*;

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn integers_are_read_as_pydantic_reads_them() {
    for (v, n) in [
        (json!(7), 7),
        (json!("7"), 7),
        (json!(" 7 "), 7),
        (json!("\t7\n"), 7),
        (json!("1_000"), 1000),
        (json!("+7"), 7),
        (json!("-7"), -7),
        (json!("07"), 7),
        (json!("7.00"), 7),
        (json!(" -7.0 "), -7),
        (json!(7.0), 7),
        (json!(true), 1),
    ] {
        assert_eq!(int(&v, "id"), Ok(n), "{v}");
    }
    for (v, why) in [
        (
            json!("x"),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!("7."),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!("7.5"),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!("1__0"),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!("_1"),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!("7_"),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!("0x10"),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!("1e3"),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!(""),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!("٣"),
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            json!(7.5),
            "Input should be a valid integer, got a number with a fractional part",
        ),
        (
            json!(1e20),
            "Unable to parse input string as an integer, exceeded maximum size",
        ),
        (json!(null), "Input should be a valid integer"),
        (json!([1]), "Input should be a valid integer"),
    ] {
        assert_eq!(int(&v, "id"), Err(Refusal::new("id", why)), "{v}");
    }
}

#[test]
fn booleans_are_read_as_pydantic_reads_them() {
    for (v, b) in [
        (json!(true), true),
        (json!(0), false),
        (json!(1), true),
        (json!(0.0), false),
        (json!("yes"), true),
        (json!("YES"), true),
        (json!("t"), true),
        (json!("on"), true),
        (json!("off"), false),
        (json!("0"), false),
    ] {
        assert_eq!(boolean(&v, "b"), Ok(b), "{v}");
    }
    let unreadable = "Input should be a valid boolean, unable to interpret input";
    for (v, why) in [
        (json!(" yes"), unreadable),
        (json!("true "), unreadable),
        (json!("2"), unreadable),
        (json!(2), unreadable),
        (json!(0.5), "Input should be a valid boolean"),
        (json!(null), "Input should be a valid boolean"),
        (json!([true]), "Input should be a valid boolean"),
    ] {
        assert_eq!(boolean(&v, "b"), Err(Refusal::new("b", why)), "{v}");
    }
}

#[test]
fn options_and_items_keep_only_their_own_keys() {
    let call = parse(
        Name::AskOperator,
        &obj(json!({
            "options": ["a", {"label": "b", "x": 1, "description": "d"}],
            "questions": [{"question": "q", "zz": 1, "multi_select": "yes"}],
            "unknown": 3
        })),
    )
    .unwrap();
    let Call::AskOperator(a) = call else { panic!() };
    assert_eq!(
        a.options,
        Some(vec![json!("a"), json!({"label": "b", "description": "d"})])
    );
    assert_eq!(
        a.questions,
        Some(vec![Item {
            question: "q".into(),
            header: None,
            options: None,
            multi_select: true
        }])
    );
    assert!(!a.multi_select);
}

#[test]
fn refusals_name_the_field_pydantic_names() {
    let refused_at = |tool, args: Value| parse(tool, &obj(args)).unwrap_err()[0].field.clone();
    assert_eq!(
        refused_at(
            Name::AskOperator,
            json!({"options": [{"description": "d"}]})
        ),
        "options.0.str"
    );
    assert_eq!(
        refused_at(Name::AskOperator, json!({"options": [3]})),
        "options.0.str"
    );
    assert_eq!(
        refused_at(Name::AskOperator, json!({"multi_select": null})),
        "multi_select"
    );
    assert_eq!(
        refused_at(Name::AskOperator, json!({"question": 3})),
        "question"
    );
    assert_eq!(
        refused_at(Name::AskOperator, json!({"questions": {"question": "x"}})),
        "questions"
    );
    assert_eq!(
        refused_at(Name::AskOperator, json!({"questions": [{}]})),
        "questions.0.question"
    );
    assert_eq!(refused_at(Name::AlertOperator, json!({})), "title");
    assert_eq!(
        refused_at(Name::AlertOperator, json!({"title": "t", "severity": null})),
        "severity"
    );
    assert_eq!(
        refused_at(Name::WithdrawQuestion, json!({"id": 1})),
        "reason"
    );
    assert_eq!(
        refused_at(Name::ListMyQuestions, json!({"include_closed": "maybe"})),
        "include_closed"
    );
}

#[test]
fn defaults_fill_what_the_call_leaves_out() {
    assert_eq!(
        parse(Name::AlertOperator, &obj(json!({"title": "t"}))),
        Ok(Call::AlertOperator {
            title: "t".into(),
            message: None,
            severity: "warning".into(),
            asked_by: None
        })
    );
    assert_eq!(
        parse(Name::ListMyQuestions, &obj(json!({}))),
        Ok(Call::ListMyQuestions {
            include_closed: false
        })
    );
    assert_eq!(
        parse(Name::CheckAnswers, &obj(json!({"x": 1}))),
        Ok(Call::CheckAnswers)
    );
    assert_eq!(
        parse(Name::WithdrawAlert, &obj(json!({"id": "3"}))),
        Ok(Call::WithdrawAlert { id: 3 })
    );
}

/// Every refusal of a call, as `field / reason` pairs.
fn all(tool: Name, args: Value) -> Vec<(String, &'static str)> {
    parse(tool, &obj(args))
        .unwrap_err()
        .into_iter()
        .map(|r| (r.field, r.why))
        .collect()
}

#[test]
fn every_refusal_is_listed_in_pydantics_order() {
    let s = |f: &str, w: &'static str| (f.to_owned(), w);
    // probed against pydantic 2.14 (the Python server's model)
    assert_eq!(
        all(Name::AskOperator, json!({"options": [5]})),
        [
            s("options.0.str", "Input should be a valid string"),
            s("options.0.Option", "Input should be a valid dictionary")
        ]
    );
    assert_eq!(
        all(
            Name::AskOperator,
            json!({"options": [{"label": 5, "description": 3}]})
        ),
        [
            s("options.0.str", "Input should be a valid string"),
            s("options.0.Option.label", "Input should be a valid string"),
            s(
                "options.0.Option.description",
                "Input should be a valid string"
            )
        ]
    );
    assert_eq!(
        all(
            Name::AskOperator,
            json!({"questions": [{"options": [null], "multi_select": "x"}], "header": 3})
        ),
        [
            s("header", "Input should be a valid string"),
            s("questions.0.question", "Field required"),
            s(
                "questions.0.options.0.str",
                "Input should be a valid string"
            ),
            s(
                "questions.0.options.0.Option",
                "Input should be a valid dictionary"
            ),
            s(
                "questions.0.multi_select",
                "Input should be a valid boolean, unable to interpret input"
            )
        ]
    );
    assert_eq!(
        all(Name::WithdrawQuestion, json!({})),
        [s("id", "Field required"), s("reason", "Field required")]
    );
    let text = refused(
        Name::WithdrawQuestion,
        &parse(Name::WithdrawQuestion, &obj(json!({}))).unwrap_err(),
    );
    assert_eq!(
        text,
        "Error executing tool withdraw_question: 2 validation errors for withdraw_questionArguments\n\
         id\n  Field required\nreason\n  Field required"
    );
}

#[test]
fn json_text_in_a_non_str_argument_is_read_as_json_first() {
    let ask = |args: Value| match parse(Name::AskOperator, &obj(args)) {
        Ok(Call::AskOperator(a)) => Ok(a),
        Ok(_) => unreachable!(),
        Err(e) => Err(e),
    };
    let a = ask(json!({"questions": "[{\"question\": \"b\"}]"})).unwrap();
    assert_eq!(a.questions.unwrap()[0].question, "b");
    let a = ask(json!({"question": "q", "options": "[\"a\", \"b\"]"})).unwrap();
    assert_eq!(a.options, Some(vec![json!("a"), json!("b")]));
    assert_eq!(ask(json!({"question": "null"})).unwrap().question, None);
    // strings and numbers stay strings
    assert_eq!(
        ask(json!({"question": "\"x\""}))
            .unwrap()
            .question
            .as_deref(),
        Some("\"x\"")
    );
    assert_eq!(
        ask(json!({"question": "7"})).unwrap().question.as_deref(),
        Some("7")
    );
    assert_eq!(
        ask(json!({"question": "[oops"}))
            .unwrap()
            .question
            .as_deref(),
        Some("[oops")
    );
    let refused_as = |args: Value| {
        ask(args)
            .unwrap_err()
            .into_iter()
            .map(|r| (r.field, r.why))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        refused_as(json!({"header": "[]"})),
        [("header".to_owned(), "Input should be a valid string")]
    );
    assert_eq!(
        refused_as(json!({"context": "{\"a\": 1}"})),
        [("context".to_owned(), "Input should be a valid string")]
    );
    assert_eq!(
        refused_as(json!({"multi_select": "null"})),
        [("multi_select".to_owned(), "Input should be a valid boolean")]
    );
    // a JSON boolean is Python's int: left a string, and that string is no int
    assert_eq!(
        all(Name::WithdrawAlert, json!({"id": "true"})),
        [(
            "id".to_owned(),
            "Input should be a valid integer, unable to parse string as an integer"
        )]
    );
    // plain `str` arguments are never pre-parsed
    assert_eq!(
        parse(
            Name::WithdrawQuestion,
            &obj(json!({"id": 1, "reason": "null"}))
        ),
        Ok(Call::WithdrawQuestion {
            id: 1,
            reason: "null".into()
        })
    );
    assert_eq!(
        parse(
            Name::AlertOperator,
            &obj(json!({"title": "[1]", "severity": "null"}))
        ),
        Ok(Call::AlertOperator {
            title: "[1]".into(),
            message: None,
            severity: "null".into(),
            asked_by: None
        })
    );
}
