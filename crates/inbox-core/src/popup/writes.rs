//! The model's own writes: each emits its effect and keeps the model's copy
//! of the database as the write leaves it (Python reads it back at its next
//! action; the next snapshot confirms it).

use super::data::Detail;
use super::event::Effect;
use super::state::Popup;

impl Popup {
    /// Store a draft (the effect, and the model's copy if the question is
    /// open); `false` if it is not open (Python `set_draft`'s rowcount).
    pub(super) fn write_draft(&mut self, qid: i64, detail: Option<Detail>) -> bool {
        self.fx.push(Effect::Draft {
            qid,
            detail: detail.clone(),
        });
        self.write_draft_quiet(qid, detail)
    }

    pub(super) fn write_draft_quiet(&mut self, qid: i64, detail: Option<Detail>) -> bool {
        match self.data.question_mut(qid).filter(|q| q.status == "open") {
            Some(q) => {
                q.draft = detail;
                true
            }
            None => false,
        }
    }

    /// Close an open question in the model's copy; `false` if it was not open.
    pub(super) fn write_status(&mut self, qid: i64, status: &str) -> bool {
        let Some(q) = self.data.question_mut(qid).filter(|q| q.status == "open") else {
            return false;
        };
        q.status = status.to_string();
        self.data.open.retain(|&id| id != qid);
        true
    }

    /// Dismiss an open question (the effect; the model's copy follows).
    pub(super) fn close_question(&mut self, qid: i64) -> bool {
        self.fx.push(Effect::Dismiss { qid });
        self.write_status(qid, "dismissed")
    }
}
