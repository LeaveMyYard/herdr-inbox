//! A key press, named as Textual names it (the golden's form): `enter`,
//! `escape`, `tab`, `shift+tab`, `up`, `down`, `left`, `right`, `home`,
//! `end`, `backspace`, `delete`, `space`, or one printable character.

use std::fmt;

/// A key the popup reacts to (anything else is [`Key::Other`]: it closes the
/// empty view and is ignored everywhere else).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// One printable character (`a`, `1`, `-`, `D`).
    Char(char),
    /// Enter.
    Enter,
    /// Escape.
    Escape,
    /// Tab.
    Tab,
    /// Shift+Tab.
    BackTab,
    /// ↑.
    Up,
    /// ↓.
    Down,
    /// ←.
    Left,
    /// →.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Space (a printable `' '` inside a text box).
    Space,
    /// Any other key, by its Textual name (`f1`, `ctrl+x`, ...).
    Other(String),
}

impl Key {
    /// Parse a Textual key name.
    pub fn parse(name: &str) -> Key {
        match name {
            "enter" => Key::Enter,
            "escape" => Key::Escape,
            "tab" => Key::Tab,
            "shift+tab" => Key::BackTab,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "backspace" => Key::Backspace,
            "delete" => Key::Delete,
            "space" => Key::Space,
            _ => {
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_control() => Key::Char(c),
                    _ => Key::Other(name.to_string()),
                }
            }
        }
    }

    /// The character a text box inserts for this key.
    pub fn printable(&self) -> Option<char> {
        match self {
            Key::Char(c) => Some(*c),
            Key::Space => Some(' '),
            _ => None,
        }
    }

    /// A digit key's value (`1`-`9`).
    pub fn digit(&self) -> Option<usize> {
        match self {
            Key::Char(c @ '1'..='9') => Some(*c as usize - '0' as usize),
            _ => None,
        }
    }

    /// Is this the character `c`?
    pub fn is(&self, c: char) -> bool {
        *self == Key::Char(c)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Key::Char(c) => return write!(f, "{c}"),
            Key::Other(name) => return f.write_str(name),
            Key::Enter => "enter",
            Key::Escape => "escape",
            Key::Tab => "tab",
            Key::BackTab => "shift+tab",
            Key::Up => "up",
            Key::Down => "down",
            Key::Left => "left",
            Key::Right => "right",
            Key::Home => "home",
            Key::End => "end",
            Key::Backspace => "backspace",
            Key::Delete => "delete",
            Key::Space => "space",
        };
        f.write_str(name)
    }
}
