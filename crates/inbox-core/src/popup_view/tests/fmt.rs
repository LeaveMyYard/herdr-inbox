//! The words built from a question, as Python builds them.

use crate::popup::{Detail, Question};
use crate::popup_view::fmt::{age, answer_line, first_line, group_name, pending_text, tab_name};

#[test]
fn first_line_strips_takes_one_line_and_cuts_with_an_ellipsis() {
    assert_eq!(first_line("  Which DB?\nmore", 80), "Which DB?");
    assert_eq!(first_line("abcdef", 4), "abc…");
    assert_eq!(first_line("abcd", 4), "abcd");
    assert_eq!(first_line(" \n ", 4), "");
    assert_eq!(first_line("a\u{2028}b", 4), "a");
}

#[test]
fn a_group_is_named_by_its_title_then_header_then_question() {
    let mut q = Question {
        question: "Which DB?\nmore".into(),
        ..Question::default()
    };
    assert_eq!(group_name(&[&q]), "Which DB?");
    q.header = Some("DB".into());
    assert_eq!(group_name(&[&q]), "DB");
    q.group_title = Some(String::new());
    assert_eq!(group_name(&[&q]), "DB");
    q.group_title = Some("Storage".into());
    assert_eq!(group_name(&[&q]), "Storage");
    assert_eq!(group_name(&[]), "");
}

#[test]
fn a_tab_is_named_by_its_label_then_pane() {
    let mut q = Question::default();
    assert_eq!(tab_name(None), "-");
    assert_eq!(tab_name(Some(&q)), "no pane");
    q.pane_id = Some("w1:p2".into());
    assert_eq!(tab_name(Some(&q)), "w1:p2");
    q.tab_label = Some("Docs".into());
    assert_eq!(tab_name(Some(&q)), "Docs");
}

#[test]
fn a_draft_reads_as_its_picks_notes_and_text() {
    let d = Detail {
        choices: vec!["Postgres".into(), "a, b".into()],
        notes: vec![("Postgres".into(), "keep it".into())],
        text: Some("  more ".into()),
    };
    assert_eq!(answer_line(&d), "Postgres (note: keep it), \"a, b\", more");
    let q = Detail {
        choices: vec!["say \"hi\"".into()],
        ..Detail::default()
    };
    assert_eq!(answer_line(&q), "\"say \\\"hi\\\"\"");
}

#[test]
fn ages_count_whole_units() {
    let t = "2026-10-17T12:00:00+00:00";
    let now = 1_792_238_400;
    assert_eq!(age(t, now), "0s");
    assert_eq!(age(t, now - 5), "0s");
    assert_eq!(age(t, now + 59), "59s");
    assert_eq!(age(t, now + 3599), "59m");
    assert_eq!(age(t, now + 7200), "2h");
    assert_eq!(age(t, now + 86400 * 3), "3d");
    assert_eq!(age("garbage", now), "");
}

#[test]
fn a_pending_answer_says_why_it_waits() {
    assert_eq!(
        pending_text("busy: typed, waiting", 4.5),
        "agent busy — will send in 16 s"
    );
    assert_eq!(
        pending_text("busy: typed, waiting", 25.0),
        "agent busy — sending at the next retry (every 10 s)"
    );
    assert_eq!(
        pending_text("blocked: prompt", 0.0),
        "waiting: agent is at a prompt/dialog"
    );
    assert_eq!(
        pending_text(
            "blocked: panel: the agent's background-task panel has focus",
            0.0
        ),
        "waiting: the agent's task panel has focus (Esc there to close it)"
    );
    assert_eq!(
        pending_text("requeued: x", 0.0),
        "⚠ not delivered earlier — will resend when the agent is idle"
    );
    assert_eq!(
        pending_text(
            "busy: typed while the agent was busy but never seen; waiting for it to be idle",
            0.0
        ),
        "⚠ not delivered — agent busy; will retry when it is idle"
    );
    assert_eq!(
        pending_text("waiting: the operator is in that pane", 0.0),
        "waiting for you to leave that pane (it is focused)"
    );
    assert_eq!(pending_text("anything else", 0.0), "anything else");
}
