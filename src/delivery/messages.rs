//! One pane's answers as the messages typed into its agent (Python
//! `service.delivery_messages`): each message fits herdr's argument cap and
//! nothing is ever truncated. An answer too long for any message goes alone
//! as a reference to `check_answers` and stays unseen.

use herdr_agent_kit::herdr_api::MAX_ARG;
use herdr_agent_kit::internal::inbox_contract::delivery::{
    Answered, delivery_text, long_answer_reference,
};

/// One message: its text, the ids of the answers in it, and whether it
/// holds them in full (`false` for a long answer's reference).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// What is typed.
    pub text: String,
    /// The answers it carries, in their order.
    pub ids: Vec<u64>,
    /// The full answers are in it: seen once it lands.
    pub full: bool,
}

/// Python `len()`: characters, not bytes.
fn len(s: &str) -> usize {
    s.chars().count()
}

/// The messages for `answers`, in Python's order: a too-long answer's
/// reference as soon as it is met, the others batched until one more would
/// pass the cap.
pub fn delivery_messages(answers: &[Answered<'_>]) -> Vec<Message> {
    let mut out = Vec::new();
    let mut batch: Vec<Answered<'_>> = Vec::new();
    let flush = |batch: &[Answered<'_>], out: &mut Vec<Message>| {
        out.push(Message {
            text: delivery_text(batch),
            ids: batch.iter().map(|a| a.id).collect(),
            full: true,
        });
    };
    for a in answers {
        if len(&delivery_text(std::slice::from_ref(a))) > MAX_ARG {
            out.push(Message {
                text: long_answer_reference(a),
                ids: vec![a.id],
                full: false,
            });
            continue;
        }
        let mut more = batch.clone();
        more.push(*a);
        if !batch.is_empty() && len(&delivery_text(&more)) > MAX_ARG {
            flush(&batch, &mut out);
            batch.clear();
        }
        batch.push(*a);
    }
    if !batch.is_empty() {
        flush(&batch, &mut out);
    }
    out
}
