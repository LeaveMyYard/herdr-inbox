//! A question, every field of the versioned read view (`questions_v6`), as
//! Python's `store.Question` reads it.

mod options;

use rusqlite::Row;
use serde_json::Value;

pub use options::{QOption, labels_json, normalize_options, options_json};

/// The read view every release up to this one can read.
pub const VIEW: &str = "questions_v6";

/// The view's columns in [`Question::from_row`]'s order, for
/// `SELECT {COLUMNS} FROM {VIEW}` (never `*`: a newer release's view may add more).
pub const COLUMNS: &str = "id, created_at, pane_id, tab_id, tab_label, session_id, cwd, asked_by, question, \
    options, context, norm, status, answer, answered_at, delivered_at, delivery_error, acknowledged_at, \
    repeat_count, delivery_attempts, header, multi_select, answer_json, group_id, draft_json, drafted_at, \
    options_json, group_title, verified_at";

/// One question.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Question {
    /// `Q{id}`.
    pub id: i64,
    /// When it was asked: bounds its forward family (#53).
    pub created_at: String,
    /// The herdr pane it was asked from.
    pub pane_id: Option<String>,
    /// That pane's tab.
    pub tab_id: Option<String>,
    /// The tab's label then.
    pub tab_label: Option<String>,
    /// The session that asked it (the row keeps it across continuations).
    pub session_id: Option<String>,
    /// The asking agent's working directory.
    pub cwd: Option<String>,
    /// The agent's role, as it gave it.
    pub asked_by: Option<String>,
    /// The question as asked.
    pub question: String,
    /// Its options: from `question_ext.options_json`, else the labels in `questions.options`.
    pub options: Option<Vec<QOption>>,
    /// Context the agent gave.
    pub context: Option<String>,
    /// The normalized question ([`super::norm::normalize`]): repeats are found by it.
    pub norm: String,
    /// `open`, `answered`, `dismissed`, `withdrawn`, ...
    pub status: String,
    /// The operator's answer (or the note of a closed question).
    pub answer: Option<String>,
    /// When it was answered (or closed).
    pub answered_at: Option<String>,
    /// When it was delivered (typed, or handed to a hook).
    pub delivered_at: Option<String>,
    /// The delivery state Python encodes in this column (a claim
    /// `sending:<at>:<token>`, `pending: …`, an error, or NULL).
    pub delivery_error: Option<String>,
    /// When the agent had it (verified in its transcript, or fetched).
    pub acknowledged_at: Option<String>,
    /// How often the agent asked it again.
    pub repeat_count: i64,
    /// Failed delivery attempts.
    pub delivery_attempts: i64,
    /// A short header the agent gave.
    pub header: Option<String>,
    /// Several options may be picked.
    pub multi_select: bool,
    /// The answer's parts (`{"choices", "notes", "text"}`), as stored.
    pub answer_json: Option<Value>,
    /// The ask_operator call it came with (its first question's id).
    pub group_id: Option<i64>,
    /// The operator's unsent answer, same shape as `answer_json`.
    pub draft_json: Option<Value>,
    /// When the draft was saved.
    pub drafted_at: Option<String>,
    /// The call's title, if the agent gave one.
    pub group_title: Option<String>,
    /// When the delivery was seen in the transcript.
    pub verified_at: Option<String>,
}

impl Question {
    /// The question from a row selected with [`COLUMNS`]. A stored JSON text that
    /// does not parse is an error, as Python's `json.loads` raised.
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Question> {
        let options_json: Option<String> = row.get(26)?;
        let options: Option<String> = row.get(9)?;
        let opts = options_json
            .filter(|s| !s.is_empty())
            .or(options.filter(|s| !s.is_empty()));
        Ok(Question {
            id: row.get(0)?,
            created_at: row.get(1)?,
            pane_id: row.get(2)?,
            tab_id: row.get(3)?,
            tab_label: row.get(4)?,
            session_id: row.get(5)?,
            cwd: row.get(6)?,
            asked_by: row.get(7)?,
            question: row.get(8)?,
            options: match opts {
                Some(text) => normalize_options(&parse(&text, 26)?),
                None => None,
            },
            context: row.get(10)?,
            norm: row.get(11)?,
            status: row.get(12)?,
            answer: row.get(13)?,
            answered_at: row.get(14)?,
            delivered_at: row.get(15)?,
            delivery_error: row.get(16)?,
            acknowledged_at: row.get(17)?,
            repeat_count: row.get(18)?,
            delivery_attempts: row.get(19)?,
            header: row.get(20)?,
            multi_select: row.get::<_, Option<i64>>(21)?.unwrap_or(0) != 0,
            answer_json: json_column(row, 22)?,
            group_id: row.get(23)?,
            draft_json: json_column(row, 24)?,
            drafted_at: row.get(25)?,
            group_title: row.get(27)?,
            verified_at: row.get(28)?,
        })
    }

    /// The group it belongs to: its call's, else its own.
    pub fn group(&self) -> i64 {
        self.group_id.filter(|g| *g != 0).unwrap_or(self.id)
    }
}

/// A JSON column: `None` for NULL or empty text, else the parsed value.
fn json_column(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<Value>> {
    match row.get::<_, Option<String>>(idx)? {
        Some(text) if !text.is_empty() => parse(&text, idx).map(Some),
        _ => Ok(None),
    }
}

fn parse(text: &str, idx: usize) -> rusqlite::Result<Value> {
    serde_json::from_str(text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}
