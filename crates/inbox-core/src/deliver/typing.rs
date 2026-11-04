//! Typing a pane's answers (the end of Python `service.deliver_batch`): one
//! message at a time, each checked again just before it is typed, and its
//! outcome taken from where the text landed.

use super::decide::{DELIVERABLE, Outcomes};
use super::reasons::{COMPOSER_DRAFT, COMPOSER_UNKNOWN, OPERATOR_IN_PANE};
use super::{Agent, Composer, Look, Mode, Outcome, Waiting};
use crate::verify::landing as landed;

/// Types each message and records where it landed.
pub(super) fn type_them(
    agent: &Agent,
    ours: &[&Waiting],
    mode: Mode,
    look: &mut impl Look,
    out: &mut Outcomes,
) {
    let ids: Vec<i64> = ours.iter().map(|q| q.id).collect();
    let busy = !DELIVERABLE.contains(&agent.status.as_str());
    for (batch, ack) in look.messages(&ids) {
        if mode.retry && !mode.force {
            // the operator may have stepped into the pane since the check above
            let again = look.agent_again(&batch);
            if again.is_none_or(|a| a.focused || a.session != agent.session) {
                out.pending(look, batch.iter().copied(), OPERATOR_IN_PANE);
                continue;
            }
        }
        if agent.kind.as_deref() == Some("claude") {
            // never type onto unsent text in Claude Code's input box
            match look.composer(&batch) {
                Composer::Empty => {}
                Composer::Draft => {
                    out.pending(look, batch.iter().copied(), COMPOSER_DRAFT);
                    continue;
                }
                Composer::Unknown => {
                    out.pending(look, batch.iter().copied(), COMPOSER_UNKNOWN);
                    continue;
                }
            }
        }
        // Python `if err:`: an empty error is a success
        if let Some(error) = look.prompt(&batch).filter(|e| !e.is_empty()) {
            for id in &batch {
                out.insert(look, *id, Outcome::Failed(error.clone()));
            }
            continue;
        }
        if !look.verify_transcript(&batch) {
            // sent, never counted as seen: check_answers still returns it
            for id in &batch {
                out.insert(look, *id, Outcome::SentUnverified);
            }
            continue;
        }
        if mode.verify_later {
            for id in &batch {
                out.insert(look, *id, Outcome::VerifyLater);
            }
            continue;
        }
        for id in batch {
            let outcome = landed(look.landing(id), ack, busy);
            out.insert(look, id, outcome);
        }
    }
}
