//! Taking back a remote action the IO layer refused
//! ([`Event::RemoteRefused`](crate::popup::Event::RemoteRefused)).
//!
//! The model decides on its copy of the mirror and moves on at once (the
//! toast, the placeholder, back to the list, perhaps the end of the popup);
//! the IO layer's write comes after. Python writes first, so it never has to
//! take anything back. When the write is refused, the model returns to the
//! state it had when the action started, so the operator ends up as if the
//! model itself had refused: an answer's view still open with what was typed,
//! the row still open. An alert's view is the one exception: Python goes back
//! to the list after a refused acknowledgement, so the row is seen as it is now.

use crate::popup::data::Snapshot;
use crate::popup::event::Severity;
use crate::popup::state::Popup;

impl Popup {
    /// Remember the state before a remote action (the state a refusal returns
    /// to). Any event but the refusal forgets it.
    pub(in crate::popup) fn checkpoint(&mut self) {
        self.undo = None;
        self.undo = Some(Box::new(self.clone()));
    }

    /// The action just taken was refused: back to before it. What the
    /// action left to run after it goes too. The timers stay as they are: a
    /// grace window the action opened goes on guarding the keys already typed
    /// at a screen that has just changed back.
    pub(in crate::popup) fn remote_refused(
        &mut self,
        notice: Option<String>,
        snapshot: Option<Snapshot>,
    ) {
        if let Some(before) = self.undo.take() {
            let (now, timers) = (self.now, self.timers);
            *self = *before;
            (self.now, self.timers) = (now, timers);
            self.queue.clear();
            self.fx.clear();
        }
        if let Some(snapshot) = snapshot {
            self.data = snapshot;
        }
        if let Some(text) = notice {
            self.notify(&text, Severity::Warning);
        }
        if self.remote_alert().is_some() {
            self.back_to_list(true);
        }
    }
}
