//! One question as Claude Code shows it (Python `QuestionView`): numbered
//! options, "Type something.", then "Chat about this" (single-select) or
//! "Submit" (multi-select); notes under options; a text box for each.

use std::collections::BTreeSet;

use super::data::{Detail, Millis, Question, py_strip};
use super::event::Severity;
use super::key::Key;
use super::ordered::Ordered;
use super::state::Msg;
use super::textbox::{BoxKey, TextBox};
use super::views::Out;

/// A row of the question view. The rows are fixed for the view's life (a
/// question's options never change), so the cursor indexes them safely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// The option at this index.
    Option(usize),
    /// "Type something."
    Other,
    /// "Chat about this" (single-select).
    Chat,
    /// "Submit" / "Next" (multi-select).
    Submit,
}

/// The open text box: a note under an option, or the typed answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Editing {
    /// A note under the option at this index.
    Note(usize, TextBox),
    /// "Type something."
    Text(TextBox),
}

impl Editing {
    /// The box.
    pub fn text_box(&self) -> &TextBox {
        match self {
            Editing::Note(_, b) | Editing::Text(b) => b,
        }
    }
}

/// The question view's state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuestionView {
    /// The question.
    pub qid: i64,
    /// Shown inside a group of several (tabs, "Tab/Arrow keys to navigate").
    pub in_group: bool,
    /// The group's last open question (its Submit row says "Submit", not "Next").
    pub last: bool,
    /// Several options may be picked.
    pub multi_select: bool,
    /// The option labels.
    pub labels: Vec<String>,
    /// The rows.
    pub rows: Vec<Row>,
    /// The row under the cursor (an index into the fixed `rows`).
    pub cursor: usize,
    /// Picked options.
    pub picked: BTreeSet<usize>,
    /// Notes per option, in the order they were written.
    pub notes: Ordered<usize, String>,
    /// The typed answer.
    pub text: Option<String>,
    /// The open box, if any (it then takes the keys).
    pub editing: Option<Editing>,
    /// The open box has the keys. Tab or Shift+Tab inside it moves Textual's
    /// focus away while the box stays open (see [`Focus`](super::Focus)).
    pub box_focused: bool,
    /// Enter, digits and Space count from then on.
    pub armed_at: Millis,
}

impl QuestionView {
    /// The view of `q`, its draft loaded, mounted at `now`.
    pub(super) fn new(q: &Question, in_group: bool, last: bool, armed_at: Millis) -> QuestionView {
        let labels = q.labels();
        let draft = q.draft.clone().unwrap_or_default();
        let index = |label: &String| labels.iter().position(|l| l == label);
        let picked: BTreeSet<usize> = draft.choices.iter().filter_map(index).collect();
        let mut notes = Ordered::default();
        for (label, note) in &draft.notes {
            if let Some(i) = index(label) {
                notes.insert(i, note.clone());
            }
        }
        let mut rows: Vec<Row> = (0..labels.len()).map(Row::Option).collect();
        rows.push(Row::Other);
        rows.push(if q.multi_select {
            Row::Submit
        } else {
            Row::Chat
        });
        let cursor = match (draft.text.is_some(), picked.first()) {
            (true, None) => labels.len(),
            (_, Some(&first)) => first,
            (false, None) => 0,
        };
        let multi_select = q.multi_select;
        QuestionView {
            qid: q.id,
            in_group,
            last,
            multi_select,
            labels,
            rows,
            cursor,
            picked,
            notes,
            text: draft.text,
            editing: None,
            box_focused: false,
            armed_at,
        }
    }

    /// Take a key; `false` if the view does not bind it (the app may).
    pub(super) fn key(&mut self, key: &Key, armed: bool, out: &mut Out) -> bool {
        if let Some(editing) = self.editing.as_mut().filter(|_| self.box_focused) {
            let b = match editing {
                Editing::Note(_, b) | Editing::Text(b) => b,
            };
            match b.key(key) {
                BoxKey::Edited => {}
                BoxKey::Submit => self.submit_box(out),
                BoxKey::Ignored if *key == Key::Escape => self.close_box(),
                BoxKey::Ignored => {} // the box has the focus: nothing else gets it
            }
            return true;
        }
        let gated = matches!(key, Key::Enter | Key::Space) || key.digit().is_some();
        if gated && !armed {
            return false;
        }
        match key {
            Key::Up | Key::Char('k') => {
                self.cursor = (self.cursor + self.rows.len() - 1) % self.rows.len()
            }
            Key::Down | Key::Char('j') => self.cursor = (self.cursor + 1) % self.rows.len(),
            Key::Enter => self.select(out),
            Key::Space => self.toggle(),
            Key::Char('n') => self.open_note(),
            Key::Char('x') => self.clear(out),
            Key::Escape if self.editing.is_some() => self.close_box(),
            Key::Escape => out.later(Msg::Back),
            _ => match key.digit() {
                Some(n) if n <= self.rows.len() && self.rows[n - 1] != Row::Submit => {
                    self.cursor = n - 1
                }
                Some(_) => {}
                None => return false,
            },
        }
        true
    }

    fn open_box(&mut self, editing: Editing) {
        self.editing = Some(editing);
        self.box_focused = true;
    }

    fn close_box(&mut self) {
        self.editing = None;
        self.box_focused = false;
    }

    fn toggle(&mut self) {
        if !self.multi_select {
            return;
        }
        match self.rows[self.cursor] {
            Row::Option(i) => {
                if !self.picked.remove(&i) {
                    self.picked.insert(i);
                }
            }
            Row::Other => self.open_box(Editing::Text(TextBox::new(
                self.text.as_deref().unwrap_or(""),
            ))),
            Row::Chat | Row::Submit => {}
        }
    }

    fn select(&mut self, out: &mut Out) {
        match self.rows[self.cursor] {
            Row::Chat => out.later(Msg::Chat(self.qid)),
            Row::Other => self.open_box(Editing::Text(TextBox::new(
                self.text.as_deref().unwrap_or(""),
            ))),
            Row::Submit if self.picked.is_empty() && self.text.is_none() => {
                out.notify("Select at least one option", Severity::Warning)
            }
            Row::Submit => self.answer(out),
            Row::Option(_) if self.multi_select => self.toggle(),
            Row::Option(i) => {
                self.picked = BTreeSet::from([i]);
                self.text = None;
                self.answer(out);
            }
        }
    }

    fn open_note(&mut self) {
        if let Row::Option(i) = self.rows[self.cursor] {
            let value = self.notes.get(&i).cloned().unwrap_or_default();
            self.open_box(Editing::Note(i, TextBox::new(&value)));
        }
    }

    fn clear(&mut self, out: &mut Out) {
        self.picked.clear();
        self.notes = Ordered::default();
        self.text = None;
        if self.in_group {
            out.later(Msg::Answered(self.qid, None));
        }
    }

    /// The view's answer as it stands (Python `_answer`).
    fn answer(&self, out: &mut Out) {
        let choices: Vec<String> = self
            .picked
            .iter()
            .map(|&i| self.labels[i].clone())
            .collect();
        let notes = self
            .notes
            .iter()
            .map(|(&i, n)| (self.labels[i].clone(), n.clone()))
            .collect();
        out.later(Msg::Answered(
            self.qid,
            Detail::make(self.text.as_deref(), choices, notes),
        ));
    }

    /// Enter in the box (Python `on_input_submitted`).
    fn submit_box(&mut self, out: &mut Out) {
        self.box_focused = false;
        let Some(editing) = self.editing.take() else {
            return;
        };
        match editing {
            Editing::Note(i, b) => {
                let value = py_strip(&b.value()).to_string();
                if value.is_empty() {
                    self.notes.retain(|k| *k != i);
                } else {
                    self.notes.insert(i, value);
                }
            }
            Editing::Text(b) => {
                let value = py_strip(&b.value()).to_string();
                if self.multi_select {
                    self.text = Some(value).filter(|v| !v.is_empty());
                } else if !value.is_empty() {
                    self.picked.clear();
                    self.text = Some(value);
                    self.answer(out);
                }
            }
        }
    }
}
