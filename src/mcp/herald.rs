//! The real [`Herald`]: herdr's notification over its socket.
//!
//! The pane's markers (its `inbox_q` count token and the `❓N` tab prefix
//! under `inbox.db.markers.lock`) are P5-G's port of `service.update_markers`;
//! until it lands they are left to the next process that refreshes them (the
//! worker's `sync_tokens`, the popup). Nothing launches this server before
//! P5-I, which needs both.

use herdr_agent_kit::herdr_api::Sanitized;
use herdr_agent_kit::herdr_api::blocking::Herdr;

use super::effects::Herald;

/// herdr as this process finds it.
pub struct Live {
    pub herdr: Herdr,
}

impl Herald for Live {
    fn notify(&self, title: &str, body: &str) {
        // Python `herdr.notify`: sound "request"; a failure is only logged there
        let _ = self.herdr.notify(
            &Sanitized::new(title),
            &Sanitized::new(body),
            Some("request"),
        );
    }

    fn markers(&self, _pane: &str) {}
}
