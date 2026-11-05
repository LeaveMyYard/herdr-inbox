use ratatui_crossterm::crossterm::event::{KeyEventState, KeyModifiers as M};

use super::*;

fn ev(code: KeyCode, modifiers: M) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn name(code: KeyCode, m: M) -> String {
    match input(&ev(code, m)) {
        Some(Input::Key(k)) => k.to_string(),
        other => format!("{other:?}"),
    }
}

#[test]
fn keys_are_named_as_textual_names_them() {
    assert_eq!(name(KeyCode::Enter, M::NONE), "enter");
    assert_eq!(name(KeyCode::Esc, M::NONE), "escape");
    assert_eq!(name(KeyCode::Tab, M::NONE), "tab");
    assert_eq!(name(KeyCode::BackTab, M::SHIFT), "shift+tab");
    assert_eq!(name(KeyCode::Tab, M::SHIFT), "shift+tab");
    assert_eq!(name(KeyCode::Char(' '), M::NONE), "space");
    assert_eq!(name(KeyCode::Char('D'), M::SHIFT), "D");
    assert_eq!(name(KeyCode::Char('1'), M::NONE), "1");
    assert_eq!(name(KeyCode::Char('x'), M::CONTROL), "ctrl+x");
    assert_eq!(name(KeyCode::Char('x'), M::ALT), "alt+x");
    assert_eq!(name(KeyCode::F(1), M::NONE), "f1");
    assert_eq!(name(KeyCode::Left, M::NONE), "left");
}

#[test]
fn ctrl_q_quits_ctrl_c_reminds_and_releases_are_nothing() {
    assert_eq!(
        input(&ev(KeyCode::Char('q'), M::CONTROL)),
        Some(Input::Quit)
    );
    assert_eq!(
        input(&ev(KeyCode::Char('c'), M::CONTROL)),
        Some(Input::HelpQuit)
    );
    let mut up = ev(KeyCode::Enter, M::NONE);
    up.kind = KeyEventKind::Release;
    assert_eq!(input(&up), None);
}
