//! What the popup shows, as the IO layer reads it from the inbox database: a
//! [`Snapshot`] when the popup opens and at every poll (Python's `store.list`,
//! `store.failed_deliveries`, `alerts.open_alerts`). The model never reads
//! anything else; between polls it keeps its own effects' writes on top
//! (drafts, dismissals, answers), as the Python popup reads them back.

/// A point on the popup's clock, in milliseconds (any epoch: only
/// differences count). The IO layer passes it with every event.
pub type Millis = u64;

/// A question's `delivery_error` prefix for an answer whose agent is gone
/// (Python `store.AGENT_GONE`).
pub const AGENT_GONE: &str = "agent gone: ";

/// A structured answer, draft or final (Python `service.make_detail`): the
/// picked option labels, a note per picked label, free text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detail {
    /// Picked option labels, in option order.
    pub choices: Vec<String>,
    /// `(label, note)` per picked label with a note, in the order the notes
    /// were written (a JSON object: keys in this order).
    pub notes: Vec<(String, String)>,
    /// Typed text ("Type something."), if any.
    pub text: Option<String>,
}

impl Detail {
    /// Python `service.make_detail`: notes only for picked labels, stripped,
    /// empty ones dropped; blank text is none; nothing picked and no text is
    /// no answer at all.
    pub fn make(
        text: Option<&str>,
        choices: Vec<String>,
        notes: Vec<(String, String)>,
    ) -> Option<Detail> {
        let notes = notes
            .into_iter()
            .filter(|(k, v)| choices.contains(k) && !py_strip(v).is_empty())
            .map(|(k, v)| (k, py_strip(&v).to_string()))
            .collect();
        let text = text
            .map(py_strip)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        if choices.is_empty() && text.is_none() {
            return None;
        }
        Some(Detail {
            choices,
            notes,
            text,
        })
    }
}

/// Python's `str.strip()`: Unicode whitespace at both ends.
pub(crate) fn py_strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// One option of a question.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QOption {
    /// What the operator picks (and what a draft stores).
    pub label: String,
    /// The dim line under it.
    pub description: Option<String>,
    /// The preview pane's text.
    pub preview: Option<String>,
}

impl QOption {
    /// An option with only a label.
    pub fn label(label: impl Into<String>) -> QOption {
        QOption {
            label: label.into(),
            ..QOption::default()
        }
    }
}

/// A question row (Python `store.Question`). The model decides with `id`,
/// `group`, `status`, `pane_id`, `options`, `multi_select`, `draft`,
/// `delivery_error` and `delivered`; the rest is for the renderer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Question {
    /// Its id (`Q<id>`).
    pub id: i64,
    /// Its group: the ask_operator call that filed it (`group_id or id`).
    pub group: i64,
    /// `open`, `answered`, `withdrawn`, `dismissed`, ...
    pub status: String,
    /// The asking agent's pane.
    pub pane_id: Option<String>,
    /// The options, in order (none: free text only).
    pub options: Vec<QOption>,
    /// Several options may be picked.
    pub multi_select: bool,
    /// The operator's unsent answer.
    pub draft: Option<Detail>,
    /// Why the answer was not delivered (or that it waits, or a claim).
    pub delivery_error: Option<String>,
    /// The answer was delivered.
    pub delivered: bool,
    /// The tab's label at ask time.
    pub tab_label: Option<String>,
    /// Who asked (`asked_by`).
    pub asked_by: Option<String>,
    /// Its header chip.
    pub header: Option<String>,
    /// The question text.
    pub question: String,
    /// Its context lines.
    pub context: Option<String>,
    /// The group's title, if the agent gave one.
    pub group_title: Option<String>,
    /// The answer as sent.
    pub answer: Option<String>,
    /// When it was asked (ISO 8601).
    pub created_at: String,
}

impl Question {
    /// `Q<id>`.
    pub fn r#ref(&self) -> String {
        format!("Q{}", self.id)
    }

    /// Its answer waits for an agent that is gone (Python `tui._gone`).
    pub fn agent_gone(&self) -> bool {
        self.delivery_error
            .as_deref()
            .is_some_and(|e| e.starts_with(AGENT_GONE))
    }

    /// The option labels, in order.
    pub fn labels(&self) -> Vec<String> {
        self.options.iter().map(|o| o.label.clone()).collect()
    }
}

/// An alert row (Python `alerts.Alert`): an agent's "please check X"
/// (`kind` `agent`), or an agent waiting at a prompt (`blocked`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alert {
    /// Its id (`A<id>`).
    pub id: i64,
    /// `agent` or `blocked`.
    pub kind: String,
    /// `open`, `acked`, `withdrawn`, `cleared`.
    pub status: String,
    /// The agent's pane.
    pub pane_id: Option<String>,
    /// `urgent`, `warning` or `info`.
    pub severity: String,
    /// Its title.
    pub title: String,
    /// Its message.
    pub message: Option<String>,
    /// The tab's label.
    pub tab_label: Option<String>,
    /// Who raised it.
    pub asked_by: Option<String>,
    /// When it was raised (ISO 8601).
    pub created_at: String,
}

/// The database as the popup reads it at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Snapshot {
    /// Every question the popup may show or name: the open ones, the failed
    /// ones, and the closed ones of groups the popup has shown (their
    /// statuses say why a row or a tab closed). A missing one was removed.
    pub questions: Vec<Question>,
    /// The open questions' ids (Python `store.list()`).
    pub open: Vec<i64>,
    /// Answers not delivered, in the database's order (Python
    /// `store.failed_deliveries()`: not hidden, not fetched, no live claim).
    pub failed: Vec<i64>,
    /// Every alert the popup may show or name.
    pub alerts: Vec<Alert>,
    /// The open alerts' ids, in order (Python `alerts.open_alerts`).
    pub open_alerts: Vec<i64>,
}

impl Snapshot {
    /// The question `id`, if the snapshot has it.
    pub fn question(&self, id: i64) -> Option<&Question> {
        self.questions.iter().find(|q| q.id == id)
    }

    /// The alert `id`, if the snapshot has it.
    pub fn alert(&self, id: i64) -> Option<&Alert> {
        self.alerts.iter().find(|a| a.id == id)
    }

    /// The group's questions in id order (Python `store.group`, with
    /// `include_closed`).
    pub fn group(&self, group: i64) -> Vec<&Question> {
        let mut qs: Vec<&Question> = self.questions.iter().filter(|q| q.group == group).collect();
        qs.sort_by_key(|q| q.id);
        qs
    }

    /// The group's open questions in id order.
    pub fn open_group(&self, group: i64) -> Vec<&Question> {
        self.group(group)
            .into_iter()
            .filter(|q| q.status == "open")
            .collect()
    }

    pub(crate) fn question_mut(&mut self, id: i64) -> Option<&mut Question> {
        self.questions.iter_mut().find(|q| q.id == id)
    }
}
