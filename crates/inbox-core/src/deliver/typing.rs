//! Typing a pane's answers (the end of Python `service.deliver_batch`): one
//! message at a time, each checked again just before it is typed, and its
//! outcome taken from where the text landed.

use super::decide::{DELIVERABLE, Outcomes};
use super::reasons::{COMPOSER_DRAFT, COMPOSER_UNKNOWN, NOT_VERIFIED, OPERATOR_IN_PANE};
use super::{Agent, Composer, Look, Mode, Outcome, Waiting};

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
        let pending = |reason: &str, out: &mut Outcomes| {
            for id in &batch {
                out.insert(*id, Outcome::Pending(reason.to_string()));
            }
        };
        if mode.retry && !mode.force {
            // the operator may have stepped into the pane since the check above
            let again = look.agent_again(&batch);
            if again.is_none_or(|a| a.focused || a.session != agent.session) {
                pending(OPERATOR_IN_PANE, out);
                continue;
            }
        }
        if agent.kind.as_deref() == Some("claude") {
            // never type onto unsent text in Claude Code's input box
            match look.composer(&batch) {
                Composer::Empty => {}
                Composer::Draft => {
                    pending(COMPOSER_DRAFT, out);
                    continue;
                }
                Composer::Unknown => {
                    pending(COMPOSER_UNKNOWN, out);
                    continue;
                }
            }
        }
        if let Some(error) = look.prompt(&batch) {
            for id in &batch {
                out.insert(*id, Outcome::Failed(error.clone()));
            }
            continue;
        }
        if !look.verify_transcript(&batch) {
            // sent, never counted as seen: check_answers still returns it
            for id in &batch {
                out.insert(*id, Outcome::SentUnverified);
            }
            continue;
        }
        if mode.verify_later {
            for id in &batch {
                out.insert(*id, Outcome::VerifyLater);
            }
            continue;
        }
        for id in batch {
            let landing = look.landing(id);
            let o = if landing.counted {
                Outcome::Delivered { ack }
            } else if landing.anywhere && landing.older_error.is_some() {
                Outcome::Failed(landing.older_error.unwrap_or_default())
            } else if busy {
                Outcome::BusyMiss
            } else {
                Outcome::Failed(NOT_VERIFIED.into())
            };
            out.insert(id, o);
        }
    }
}
