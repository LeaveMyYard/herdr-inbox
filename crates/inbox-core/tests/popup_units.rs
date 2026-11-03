//! The popup model's small pieces, beside the golden replay
//! (tests/popup_transitions.rs): key names, entry names, answers, the text
//! box, and the rules no Python scenario reaches.

use inbox_core::popup::{
    Detail, Effect, Entry, Event, Key, Open, Popup, QOption, Question, Snapshot, Tab, TextBox,
};

#[test]
fn key_names_are_textuals() {
    for name in [
        "enter",
        "escape",
        "tab",
        "shift+tab",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "backspace",
        "delete",
        "space",
        "a",
        "1",
        "-",
        "D",
        "f1",
        "ctrl+x",
    ] {
        assert_eq!(Key::parse(name).to_string(), name);
    }
    assert_eq!(Key::parse("a"), Key::Char('a'));
    assert_eq!(Key::parse("ctrl+x"), Key::Other("ctrl+x".into()));
    assert_eq!(Key::parse("7").digit(), Some(7));
    assert_eq!(Key::parse("0").digit(), None);
    assert_eq!(Key::Space.printable(), Some(' '));
    assert_eq!(Key::Enter.printable(), None);
}

#[test]
fn entry_names_round_trip() {
    for e in [
        Entry::Group(2),
        Entry::Failed(5),
        Entry::Gone,
        Entry::Alert(1),
    ] {
        assert_eq!(Entry::parse(&e.to_string()), Some(e));
    }
    assert_eq!(Entry::parse("remote:m|1"), None);
    assert_eq!(Tab::Question(3).to_string(), "q:3");
    assert_eq!(Tab::Submit.to_string(), "submit");
}

#[test]
fn an_answer_is_made_as_python_makes_it() {
    let notes = vec![
        ("a".to_string(), "  keep  ".to_string()),
        ("b".to_string(), "unpicked".to_string()),
        ("c".to_string(), "   ".to_string()),
    ];
    let d = Detail::make(Some("  "), vec!["a".into(), "c".into()], notes).unwrap();
    assert_eq!(d.notes, vec![("a".to_string(), "keep".to_string())]);
    assert_eq!(d.text, None);
    assert_eq!(Detail::make(Some(" \n"), vec![], vec![]), None);
    assert_eq!(
        Detail::make(Some(" hi "), vec![], vec![])
            .unwrap()
            .text
            .as_deref(),
        Some("hi")
    );
}

#[test]
fn the_text_box_edits_like_textuals_input() {
    let mut b = TextBox::new("");
    for k in [
        "a",
        "b",
        "left",
        "x",
        "right",
        "backspace",
        "home",
        "delete",
        "end",
        "space",
    ] {
        b.key(&Key::parse(k));
    }
    assert_eq!((b.value().as_str(), b.cursor()), ("x ", 2));
    let mut b = TextBox::new("héllo"); // opened on text: all of it selected
    assert!(b.selected());
    b.key(&Key::Char('z'));
    assert_eq!((b.value().as_str(), b.cursor()), ("z", 1));
    let mut b = TextBox::new("ab");
    b.key(&Key::Left);
    b.key(&Key::Char('x'));
    assert_eq!((b.value().as_str(), b.cursor()), ("xab", 1));
}

fn question(id: i64, pane: Option<&str>) -> Question {
    Question {
        id,
        group: id,
        status: "open".into(),
        pane_id: pane.map(str::to_string),
        options: vec![QOption::label("a")],
        ..Question::default()
    }
}

fn opened(qs: Vec<Question>) -> (Popup, Vec<Effect>) {
    let mut snapshot = Snapshot::default();
    snapshot.open = qs.iter().map(|q| q.id).collect();
    snapshot.questions = qs;
    Popup::open(
        Open {
            focus_pane: None,
            snapshot,
            deeplink: None,
        },
        10_000,
    )
}

#[test]
fn g_on_a_question_without_a_pane_says_so() {
    let (mut p, _) = opened(vec![question(1, None), question(2, Some("w1:p2"))]);
    let fx = p.handle(Event::Key(Key::Char('g')), 11_000);
    assert_eq!(
        fx,
        vec![Effect::Notify {
            text: "could not focus: no pane".into(),
            severity: inbox_core::popup::Severity::Warning
        }]
    );
}

#[test]
fn nothing_happens_after_the_popup_closed() {
    let (mut p, _) = opened(vec![]);
    assert_eq!(
        p.handle(Event::Key(Key::Char('x')), 11_000),
        vec![Effect::Exit]
    );
    assert!(!p.running());
    assert!(p.handle(Event::Key(Key::Enter), 12_000).is_empty());
    assert!(
        p.handle(Event::Poll(Snapshot::default()), 13_000)
            .is_empty()
    );
}

#[test]
fn a_second_box_replaces_one_that_lost_the_focus() {
    // Python crashes here (Textual DuplicateIds), so the golden cannot hold it.
    let mut q = question(1, Some("w1:p1"));
    q.options = vec![QOption::label("a"), QOption::label("b")];
    let mut q2 = question(2, Some("w1:p1"));
    q2.group = 1;
    let (mut p, _) = opened(vec![q, q2]);
    let mut at = 11_000;
    for k in ["n", "x", "shift+tab", "n", "y", "enter"] {
        at += 600;
        p.handle(Event::Key(Key::parse(k)), at);
    }
    let inbox_core::popup::View::Question(v) = p.view() else {
        panic!("{:?}", p.view())
    };
    assert!(v.editing.is_none());
    assert_eq!(v.notes.get(&0).map(String::as_str), Some("y")); // the new box's text replaced "x"
}
