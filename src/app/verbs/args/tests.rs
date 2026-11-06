use super::{Entry, Parsed, Request, Verb, canonical_id, parse};

fn run(words: &[&str]) -> Parsed {
    parse(&words.iter().map(|w| (*w).to_string()).collect::<Vec<_>>())
}

fn request(words: &[&str]) -> Request {
    match run(words) {
        Parsed::Run(r) => r,
        other => panic!("{words:?}: {other:?}"),
    }
}

fn usage(words: &[&str]) {
    assert!(matches!(run(words), Parsed::Usage(_)), "{words:?}");
}

const DETAIL: &str = r#"{"choices":[],"notes":{},"text":"x"}"#;

#[test]
fn ids_are_canonical_decimal_up_to_the_safe_integer() {
    assert_eq!(canonical_id("1"), Some(1));
    assert_eq!(canonical_id("9007199254740991"), Some(9007199254740991));
    for bad in [
        "",
        "0",
        "012",
        "+1",
        "-1",
        "1.0",
        "1e3",
        "Q1",
        "0x10",
        " 1",
        "9007199254740992",
        "99999999999999999999",
    ] {
        assert_eq!(canonical_id(bad), None, "{bad}");
    }
}

#[test]
fn arguments_come_in_any_order_and_a_repeat_of_a_question_counts_once() {
    let r = request(&[
        "answer",
        "--json",
        "--question",
        "2",
        "--token",
        "t",
        "--question",
        "3",
        "--entry",
        "group:2",
        "--question",
        "2",
    ]);
    assert_eq!(r.verb, Verb::Answer);
    assert_eq!(r.entry, Entry::Group(2));
    assert_eq!(r.token, "t");
    assert_eq!(r.questions, [2, 3]);
}

#[test]
fn every_verb_needs_json_an_entry_and_a_token() {
    usage(&["ack", "--entry", "alert:1", "--token", "t"]);
    usage(&["ack", "--token", "t", "--json"]);
    usage(&["ack", "--entry", "alert:1", "--json"]);
    usage(&["ack", "--entry", "alert:01", "--token", "t", "--json"]);
    usage(&["ack", "--entry", "gone", "--token", "t", "--json"]);
    usage(&["nothing", "--entry", "alert:1", "--token", "t", "--json"]);
    usage(&[]);
}

#[test]
fn an_option_may_come_once_and_only_where_its_verb_takes_it() {
    let base = ["--entry", "group:1", "--token", "t", "--json"];
    let with = |verb: &'static str, more: &[&'static str]| {
        let mut w = vec![verb];
        w.extend(base);
        w.extend(more);
        w
    };
    usage(&with("answer", &[]));
    usage(&with("answer", &["--question", "1", "--clear"]));
    usage(&with(
        "answer",
        &["--question", "1", "--question", "2", "--detail", DETAIL],
    ));
    usage(&with("draft", &["--question", "1"]));
    usage(&with(
        "draft",
        &["--question", "1", "--clear", "--detail", DETAIL],
    ));
    usage(&with(
        "draft",
        &["--question", "1", "--question", "2", "--clear"],
    ));
    usage(&with("dismiss", &["--question", "1", "--question", "2"]));
    usage(&with("dismiss", &["--detail", DETAIL]));
    usage(&with("retry", &["--question", "1"]));
    usage(&with("retry", &["--force"]));
    usage(&with("ack", &["--clear"]));
    usage(&with("ack", &["--json"]));
    usage(&with("ack", &["--token", "u"]));
}

/// The verb's only way to force is the database's host: a mod cannot ask.
#[test]
fn there_is_no_force_argument() {
    usage(&[
        "retry", "--entry", "failed:1", "--token", "t", "--json", "--force",
    ]);
}

#[test]
fn a_detail_must_be_a_detail_of_at_most_64_kib() {
    let at = |detail: &str| {
        run(&[
            "answer",
            "--entry",
            "group:1",
            "--token",
            "t",
            "--json",
            "--question",
            "1",
            "--detail",
            detail,
        ])
    };
    assert!(matches!(at(DETAIL), Parsed::Run(_)));
    for bad in [
        "not json",
        "[]",
        "{}",
        r#"{"choices":[],"notes":{}}"#,
        r#"{"choices":[1],"notes":{},"text":null}"#,
        r#"{"choices":[],"notes":{"a":1},"text":null}"#,
        r#"{"choices":[],"notes":{},"text":1}"#,
        r#"{"choices":[],"notes":{},"text":null,"more":"words"}"#,
    ] {
        assert!(matches!(at(bad), Parsed::Usage(_)), "{bad}");
    }
    let text = "x".repeat(64 * 1024);
    let big = format!(r#"{{"choices":[],"notes":{{}},"text":"{text}"}}"#);
    assert!(matches!(at(&big), Parsed::Usage(_)));
}

/// A stray word on the command line may be an answer: usage never repeats it.
#[test]
fn usage_never_echoes_a_word_of_the_command() {
    let Parsed::Usage(why) = run(&["answer", "Answer Text 567"]) else {
        panic!();
    };
    assert!(!why.contains("Answer"), "{why}");
}

#[test]
fn a_remote_row_and_a_verb_it_does_not_take_read_as_such() {
    let on = |verb: &'static str, entry: &'static str| {
        request(&[verb, "--entry", entry, "--token", "t", "--json"]).not_for_this_row()
    };
    assert_eq!(on("ack", "alert:1"), None);
    assert_eq!(on("stop-showing", "failed:1"), None);
    assert_eq!(on("dismiss", "group:1"), None);
    assert_eq!(
        on("ack", "failed:1"),
        Some("This row does not take that action")
    );
    assert_eq!(
        on("retry", "group:1"),
        Some("This row does not take that action")
    );
    assert_eq!(
        on("dismiss", "alert:1"),
        Some("This row does not take that action")
    );
    for remote in ["remote:m|d|1", "routbox:r", "rproblem:m|1", "ralert:m|1"] {
        assert_eq!(on("ack", remote), Some("Answer this in the herdr popup"));
    }
}

#[test]
fn show_takes_a_question_and_json_is_its_own_choice() {
    assert_eq!(
        run(&["show", "--question", "4"]),
        Parsed::Show {
            question: 4,
            json: false
        }
    );
    assert_eq!(
        run(&["show", "--json", "--question", "4"]),
        Parsed::Show {
            question: 4,
            json: true
        }
    );
    usage(&["show"]);
    usage(&["show", "--question", "4", "--question", "5"]);
    usage(&["show", "--question", "4", "--token", "t"]);
    usage(&["show", "--question", "4", "--entry", "group:4"]);
}

#[test]
fn help_is_alone() {
    assert_eq!(run(&["--help"]), Parsed::Help);
    usage(&["--help", "--json"]);
}
