use super::*;
use crate::cli::specs::{ANSWER, LIST};

fn words(line: &[&str]) -> Vec<String> {
    line.iter().map(|w| w.to_string()).collect()
}

fn run(cmd: &Command, line: &[&str]) -> Args {
    match parse(cmd, &words(line)) {
        Parsed::Run(args) => args,
        other => panic!("{line:?}: {other:?}"),
    }
}

#[test]
fn negative_numbers_a_lone_dash_and_spaced_words_are_positionals() {
    let a = run(&ANSWER, &["1", "-5", "-", "-x y", "-1.5"]);
    assert_eq!(a.values, vec![vec!["1"], vec!["-5", "-", "-x y", "-1.5"]]);
}

#[test]
fn a_double_dash_ends_the_options_once() {
    let a = run(&ANSWER, &["--", "2", "--all", "--"]);
    assert_eq!(a.values, vec![vec!["2"], vec!["--all", "--"]]);
}

#[test]
fn a_flag_is_known_by_any_unambiguous_prefix() {
    assert!(run(&LIST, &["--a"]).has("--all"));
    assert!(run(&LIST, &["--all", "--al"]).has("--all"));
    assert!(!run(&LIST, &[]).has("--all"));
}

#[test]
fn words_after_the_positionals_are_left_over() {
    let Parsed::Exit { code, stderr, .. } = parse(&ANSWER, &words(&["1", "go", "--x", "more"]))
    else {
        panic!()
    };
    assert_eq!(code, 2);
    assert!(
        stderr.ends_with("inbox: error: unrecognized arguments: --x more\n"),
        "{stderr}"
    );
}
