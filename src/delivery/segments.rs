//! The exact strings a delivery of one answer carries (Python
//! `service.expected_segments`).

use herdr_agent_kit::herdr_api::clean;
use herdr_agent_kit::internal::inbox_contract::delivery::{
    Answered, answer_line, delivery_text, long_answer_reference,
};

use crate::store::Question;

/// The index of the long-answer reference in [`expected_segments`]: finding
/// only it delivers the pointer, not the answer itself.
pub use inbox_core::REFERENCE_INDEX;

/// No cap: `herdr.clean(text, limit=10**9)`.
const NO_LIMIT: usize = 1_000_000_000;

/// The question as its delivery shows it. A NULL answer reads `None`, as
/// Python's f-strings print it.
pub fn answered(q: &Question) -> Answered<'_> {
    Answered {
        id: u64::try_from(q.id).unwrap_or(0),
        question: &q.question,
        answer: q.answer.as_deref().unwrap_or("None"),
    }
}

/// The single message, the batch segment and the long-answer reference of
/// `q`, as herdr types them (`clean`: tabs, newlines, U+2028 become spaces):
/// what a transcript must show for `q` to count as landed.
pub fn expected_segments(q: &Question) -> Vec<String> {
    let a = answered(q);
    [
        delivery_text(&[a]),
        answer_line(&a),
        long_answer_reference(&a),
    ]
    .iter()
    .map(|s| clean(s, NO_LIMIT))
    .collect()
}

#[cfg(test)]
mod tests;
