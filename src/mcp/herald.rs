//! The real [`Herald`]: herdr over its socket through the CLI's surface
//! ([`crate::surface::live::Live`]), and the pane markers as the CLI refreshes
//! them ([`Markers`], Python `service.update_markers_now`): the pane's
//! `inbox_q` count token and the `❓N` prefix of its tab, under the tab-label
//! lock.
//!
//! As in Python's background job, each refresh opens its own store (busy
//! timeout 5 s, never migrating). Markers are cosmetic: a failure is warned
//! about on stderr (Python logs it) and never reaches the agent.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::plugin_env::config::ConfigFile;

use super::effects::Herald;
use crate::store::Store;
use crate::surface::Surface;
use crate::surface::live::Live as Surfaced;
use crate::surface::markers::Markers;
use crate::tabs::lock::LOCK_TIMEOUT;

/// The busy timeout of the store a marker refresh opens (Python `Store(path, timeout=5)`).
pub const MARKERS_STORE_TIMEOUT: Duration = Duration::from_secs(5);

fn warn(msg: &str) {
    let _ = writeln!(std::io::stderr(), "herdr-inbox: {msg}");
}

/// herdr and the plugin's settings, as the effects' one thread uses them.
pub struct Live {
    state: Mutex<(Surfaced, ConfigFile)>,
    db: PathBuf,
}

impl Live {
    /// The herald of a server whose herdr is `herdr`, database `db` and
    /// plugin settings `config`.
    pub fn new(herdr: Herdr, db: PathBuf, config: ConfigFile) -> Live {
        Live {
            state: Mutex::new((Surfaced::new(herdr, warn), config)),
            db,
        }
    }
}

impl Herald for Live {
    fn notify(&self, title: &str, body: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.0.notify(title, body);
        }
    }

    fn markers(&self, pane: &str) {
        let store = match Store::open(&self.db, MARKERS_STORE_TIMEOUT) {
            Ok(store) => store,
            Err(e) => {
                return warn(&format!(
                    "could not refresh the inbox markers for {pane}: {e}"
                ));
            }
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let (surface, config) = &mut *state;
        let mut warned = |m: &str| warn(m);
        Markers {
            store: &store,
            db_path: &self.db,
            herdr: surface,
            config,
            lock_wait: LOCK_TIMEOUT,
            warn: &mut warned,
        }
        .update(Some(pane));
    }
}
