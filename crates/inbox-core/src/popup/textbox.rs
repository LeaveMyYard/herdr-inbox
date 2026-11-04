//! The inline text box under an option (a note) or under "Type something."
//! (Textual's `Input`, as the popup uses it): printable keys insert at the
//! cursor, ←/→/Home/End move it, Backspace/Delete remove, Enter submits.
//! A box opened on existing text starts with all of it selected (Textual's
//! select-on-focus): typing or deleting replaces it, ← and → go to its ends.

use super::key::Key;

/// The box's text and cursor (a character index).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextBox {
    chars: Vec<char>,
    cursor: usize,
    selected: bool,
}

/// What a key did to the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxKey {
    /// It edited the text or moved the cursor.
    Edited,
    /// Enter: submit the value.
    Submit,
    /// Not a key the box takes.
    Ignored,
}

impl TextBox {
    /// A box holding `value`, the cursor at its end.
    pub fn new(value: &str) -> TextBox {
        let chars: Vec<char> = value.chars().collect();
        let cursor = chars.len();
        let selected = !chars.is_empty();
        TextBox {
            chars,
            cursor,
            selected,
        }
    }

    /// The text.
    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    /// The cursor, in characters from the start.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// All of the text is selected (the box was opened on it, untouched since).
    pub fn selected(&self) -> bool {
        self.selected
    }

    /// Take a key.
    pub fn key(&mut self, key: &Key) -> BoxKey {
        let Some(action) = Action::of(key) else {
            return BoxKey::Ignored;
        };
        if action == Action::Submit {
            return BoxKey::Submit;
        }
        let n = self.chars.len();
        if std::mem::take(&mut self.selected) {
            match action {
                Action::Left => return self.moved(0),
                Action::Right => return self.moved(n),
                Action::Insert(_) => self.chars.clear(),
                Action::Home | Action::End | Action::LeftWord | Action::RightWord => {}
                _ => return self.delete(0, n), // every delete takes the selection
            }
            self.cursor = self.cursor.min(self.chars.len());
        }
        let (n, at) = (self.chars.len(), self.cursor);
        match action {
            Action::Insert(c) => {
                self.chars.insert(at, c);
                self.cursor += 1;
            }
            Action::Left => self.cursor = at.saturating_sub(1),
            Action::Right => self.cursor = (at + 1).min(n),
            Action::Home => self.cursor = 0,
            Action::End => self.cursor = n,
            Action::LeftWord => self.cursor = self.word_start_before(at),
            Action::RightWord => self.cursor = self.word_start_after(at).unwrap_or(n),
            Action::DeleteLeft => return self.delete(at.saturating_sub(1), at),
            Action::DeleteRight => return self.delete(at, (at + 1).min(n)),
            Action::DeleteLeftWord => return self.delete(self.word_start_before(at), at),
            Action::DeleteRightWord => {
                return self.delete(at, self.word_start_after(at).unwrap_or(n));
            }
            Action::DeleteLeftAll => return self.delete(0, at),
            Action::DeleteRightAll => return self.delete(at, n),
            Action::Submit => {}
        }
        BoxKey::Edited
    }

    fn moved(&mut self, to: usize) -> BoxKey {
        self.cursor = to;
        BoxKey::Edited
    }

    fn delete(&mut self, from: usize, to: usize) -> BoxKey {
        self.chars.drain(from..to);
        self.moved(from)
    }

    /// A word starts at `i`: a word character after a non-word one
    /// (Textual's `(?<=\W)\w`, matched within `[lo, hi)`).
    fn word_start(&self, i: usize, lo: usize) -> bool {
        i > lo && is_word(self.chars[i]) && !is_word(self.chars[i - 1])
    }

    /// The last word start before `at`, else 0 (Textual `cursor_left_word`).
    fn word_start_before(&self, at: usize) -> usize {
        (0..at).rev().find(|&i| self.word_start(i, 0)).unwrap_or(0)
    }

    /// The first word start after `at`, searched in the text from `at` on, so
    /// `at` itself never counts (Textual `cursor_right_word`).
    fn word_start_after(&self, at: usize) -> Option<usize> {
        (at..self.chars.len()).find(|&i| self.word_start(i, at))
    }
}

/// Python's `\w`: letters, digits (any script) and the underscore.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The editing actions of Textual's `Input` the popup's boxes get. Shift
/// selections and the clipboard (`ctrl+x`/`c`/`v`) are not modelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Insert(char),
    Left,
    Right,
    Home,
    End,
    LeftWord,
    RightWord,
    DeleteLeft,
    DeleteRight,
    DeleteLeftWord,
    DeleteRightWord,
    DeleteLeftAll,
    DeleteRightAll,
    Submit,
}

impl Action {
    fn of(key: &Key) -> Option<Action> {
        if let Some(c) = key.printable() {
            return Some(Action::Insert(c));
        }
        Some(match key {
            Key::Enter => Action::Submit,
            Key::Left => Action::Left,
            Key::Right => Action::Right,
            Key::Home => Action::Home,
            Key::End => Action::End,
            Key::Backspace => Action::DeleteLeft,
            Key::Delete => Action::DeleteRight,
            Key::Other(name) => match name.as_str() {
                "ctrl+a" => Action::Home,
                "ctrl+e" => Action::End,
                "ctrl+d" => Action::DeleteRight,
                "ctrl+left" => Action::LeftWord,
                "ctrl+right" => Action::RightWord,
                "ctrl+w" => Action::DeleteLeftWord,
                "ctrl+backspace" | "alt+backspace" => Action::DeleteRightWord,
                "ctrl+u" | "super+backspace" => Action::DeleteLeftAll,
                "ctrl+k" => Action::DeleteRightAll,
                _ => return None,
            },
            _ => return None,
        })
    }
}
