//! The inline text box under an option (a note) or under "Type something."
//! (Textual's `Input`, as the popup uses it): printable keys insert at the
//! cursor, ←/→/Home/End move it, Backspace/Delete remove, Enter submits.

use super::key::Key;

/// The box's text and cursor (a character index).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextBox {
    chars: Vec<char>,
    cursor: usize,
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
        TextBox { chars, cursor }
    }

    /// The text.
    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    /// The cursor, in characters from the start.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Take a key.
    pub fn key(&mut self, key: &Key) -> BoxKey {
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
}
