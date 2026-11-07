//! Typing a pane's answers (the end of Python `service.deliver_batch`): one
//! message at a time, each checked again just before it is typed, and its
//! outcome taken from where the text landed.

use super::decide::{DELIVERABLE, Outcomes};
use super::reasons::OPERATOR_IN_PANE;
use super::{Agent, Look, Mode, Outcome, Prompted, Waiting};
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
        // never type onto unsent text in Claude Code's input box, for any kind
        // (#96): a renamed or misreported kind must not switch the guard off.
        // The last herdr read before the prompt; a keystroke between the two
        // calls stays unseen (#99, accepted: herdr has no conditional prompt)
        // (only Claude Code's box must be read: another harness draws none)
        if let Some(reason) = agent.held_by(look.composer(&batch)) {
            out.pending(look, batch.iter().copied(), reason);
            continue;
        }
        match look.prompt(&batch) {
            Prompted::Sent => {}
            // Python `if err:`: an empty error is a success
            Prompted::Failed(error) if error.is_empty() => {}
            Prompted::Failed(error) => {
                for id in &batch {
                    out.insert(look, *id, Outcome::Failed(error.clone()));
                }
                continue;
            }
            Prompted::Refused(reason) => {
                out.pending(look, batch.iter().copied(), &reason);
                continue;
            }
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
