//! A question as the hook paths read it: the columns of the versioned read
//! view (`questions_v6`) that delivery uses (Python `store.Question`).

use rusqlite::Row;

/// The read view every release up to this one can read.
pub const VIEW: &str = "questions_v6";

/// The columns [`Question::from_row`] reads, for `SELECT {COLUMNS} FROM {VIEW}`.
pub const COLUMNS: &str =
    "id, created_at, session_id, question, answer, answered_at, delivery_error";

/// One question: the fields a delivery reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// `Q{id}`.
    pub id: i64,
    /// When it was asked: bounds its forward family (#53).
    pub created_at: String,
    /// The session that asked it (the row keeps it across continuations).
    pub session_id: Option<String>,
    /// The question as asked.
    pub question: String,
    /// The operator's answer.
    pub answer: Option<String>,
    /// When it was answered: an older transcript record never counts as its
    /// delivery.
    pub answered_at: Option<String>,
    /// The delivery state Python encodes in this column (a claim
    /// `sending:<at>:<token>`, `pending: …`, an error, or NULL).
    pub delivery_error: Option<String>,
}

impl Question {
    /// The question from a row selected with [`COLUMNS`].
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Question> {
        Ok(Question {
            id: row.get(0)?,
            created_at: row.get(1)?,
            session_id: row.get(2)?,
            question: row.get(3)?,
            answer: row.get(4)?,
            answered_at: row.get(5)?,
            delivery_error: row.get(6)?,
        })
    }
}
