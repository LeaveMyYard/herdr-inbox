//! `g`, go to the agent (Python `_focus_pane`'s thread): off the popup's
//! thread, because it asks herdr. An agent waiting at a prompt would take
//! the operator's next key as its answer, so a `g` the model did not judge
//! deliberate ([`Effect::GoTo`]'s `graced`) first asks herdr whether the
//! agent is blocked, and if it is only says so. Otherwise herdr focuses the
//! agent's pane and the model hears how it went ([`Back::Focused`]: it
//! closes the popup, or shows herdr's error).
//!
//! [`Effect::GoTo`]: inbox_core::popup::Effect::GoTo

use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use inbox_core::popup::Severity;

use crate::tui::back::Back;
use crate::tui::toasts::Toast;

/// What a `g` waits for at a prompt says.
pub const AT_PROMPT: &str = "That agent is waiting at a prompt: pause, then press g";

/// herdr's side of going to an agent.
pub trait Agents: Send + Sync {
    /// herdr reports the agent in `pane` as blocked (at a prompt or dialog).
    fn blocked(&self, pane: &str) -> bool;
    /// Focuses the agent in `pane`; herdr's error if that failed.
    fn focus(&self, pane: &str) -> Option<String>;
}

/// Goes to the agent in `pane` in the background; reports on `back`.
pub fn go_to(pane: String, graced: bool, agents: Arc<dyn Agents>, back: Sender<Back>) {
    let run = move || {
        if !graced && agents.blocked(&pane) {
            let _ = back.send(Back::Toast(Toast::new(AT_PROMPT, Severity::Information)));
            return;
        }
        let error = agents.focus(&pane);
        let _ = back.send(Back::Focused { pane, error }); // the popup may be gone by now
    };
    // a thread that cannot start: nothing happens, as a herdr that does not answer
    let _ = thread::Builder::new().name("inbox-focus".into()).spawn(run);
}
