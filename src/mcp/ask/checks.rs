//! `ask_operator`'s checks before anything is filed (Python
//! `server.ask_operator`'s opening lines): `question` or `questions`, not both
//! and not neither; at most [`MAX_QUESTIONS`] questions and [`MAX_OPTIONS`]
//! options each. A refusal is an error receipt, not a tool error.

use serde_json::Value;

use crate::mcp::args::{Ask, Item};
use crate::mcp::receipts;

/// AskUserQuestion's limit per call (Python `service.MAX_QUESTIONS`).
pub const MAX_QUESTIONS: usize = 4;
/// Number keys 1-9 (Python `service.MAX_OPTIONS`).
pub const MAX_OPTIONS: usize = 9;

/// The call's questions, in order: `questions`, or the one `question` with
/// the call's `options`, `header` and `multi_select`. `Err`: the receipt that
/// refuses the call.
pub fn items(ask: &Ask) -> Result<Vec<Item>, Value> {
    // Python `bool(question) == bool(questions)`: an empty text or list is none
    let one = ask.question.as_deref().is_some_and(|q| !q.is_empty());
    let many = ask.questions.as_ref().is_some_and(|qs| !qs.is_empty());
    if one == many {
        return Err(receipts::error(
            "Pass either `question` or `questions` (not both).",
        ));
    }
    let items = match &ask.questions {
        Some(qs) if many => qs.clone(),
        _ => vec![Item {
            question: ask.question.clone().unwrap_or_default(),
            header: ask.header.clone(),
            options: ask.options.clone(),
            multi_select: ask.multi_select,
        }],
    };
    if items.len() > MAX_QUESTIONS {
        return Err(receipts::error(format!(
            "At most {MAX_QUESTIONS} questions per call."
        )));
    }
    if items
        .iter()
        .any(|i| i.options.as_ref().is_some_and(|o| o.len() > MAX_OPTIONS))
    {
        return Err(receipts::error(format!(
            "At most {MAX_OPTIONS} options; ask fewer or split the question."
        )));
    }
    Ok(items)
}
