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
        if std::mem::take(&mut self.selected) {
            match key {
                Key::Left => return self.moved(0),
                Key::Right => return self.moved(self.chars.len()),
                Key::Backspace | Key::Delete => return self.moved_after_clear(),
                _ if key.printable().is_some() => {
                    self.chars.clear();
                    self.cursor = 0;
                }
                Key::Home | Key::End => {}
                _ => self.selected = true, // Enter, Escape, ...: the selection stays
            }
        }
        if let Some(c) = key.printable() {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
            return BoxKey::Edited;
        }
        match key {
            Key::Enter => return BoxKey::Submit,
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = self.chars.len(),
            Key::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.chars.remove(self.cursor);
            }
            Key::Delete if self.cursor < self.chars.len() => {
                self.chars.remove(self.cursor);
            }
            Key::Backspace | Key::Delete => {}
            _ => return BoxKey::Ignored,
        }
        BoxKey::Edited
    }

    fn moved(&mut self, to: usize) -> BoxKey {
        self.cursor = to;
        BoxKey::Edited
    }

    fn moved_after_clear(&mut self) -> BoxKey {
        self.chars.clear();
        self.moved(0)
    }
}
