//! A terminal key as the popup names it ([`Input`]): crossterm's key events
//! in Textual's names (`enter`, `shift+tab`, `space`, `D`, `ctrl+x`), the
//! names the model and its transition golden use.

use inbox_core::popup::Key;
use ratatui_crossterm::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::tui::screen::Input;

/// The input for `ev`; `None` for a key release (a held key repeats as
/// presses, as Textual sees it).
pub fn input(ev: &KeyEvent) -> Option<Input> {
    if ev.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let alt = ev.modifiers.contains(KeyModifiers::ALT);
    if ctrl
        && !alt
        && let KeyCode::Char(c) = ev.code
    {
        return Some(match c.to_ascii_lowercase() {
            'q' => Input::Quit,
            'c' => Input::HelpQuit,
            c => Input::Key(Key::Other(format!("ctrl+{c}"))),
        });
    }
    let key = match ev.code {
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Escape,
        KeyCode::Tab if ev.modifiers.contains(KeyModifiers::SHIFT) => Key::BackTab,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Char(' ') if !ctrl && !alt => Key::Space,
        KeyCode::Char(c) if !ctrl && !alt => Key::Char(c),
        KeyCode::Char(c) => Key::Other(format!("{}{c}", prefix(ev.modifiers))),
        KeyCode::F(n) => Key::Other(format!("{}f{n}", prefix(ev.modifiers))),
        other => Key::Other(format!("{other:?}").to_lowercase()),
    };
    Some(Input::Key(key))
}

fn prefix(m: KeyModifiers) -> String {
    let mut p = String::new();
    for (flag, name) in [
        (KeyModifiers::CONTROL, "ctrl+"),
        (KeyModifiers::ALT, "alt+"),
        (KeyModifiers::SHIFT, "shift+"),
    ] {
        if m.contains(flag) {
            p.push_str(name);
        }
    }
    p
}

#[cfg(test)]
mod tests;
