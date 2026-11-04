//! One pane's marks (Python `service.update_markers_now`): its `$inbox_q`
//! token and the `❓N` prefix of the tab it is in now.
//!
//! The token is per pane and idempotent: no lock. The tab label update
//! reads the count, re-reads the label and renames under the tab-label lock
//! ([`crate::tabs::lock`]), so concurrent processes cannot leave a stale
//! count and a slow herdr never blocks database writers.

use std::path::Path;
use std::time::Duration;

use herdr_agent_kit::plugin_env::config::ConfigFile;

use super::Surface;
use crate::store::Store;
use crate::tabs::apply::apply;
use crate::tabs::label::tab_counts;
use crate::tabs::lock::marker_lock;
use crate::tabs::settings;

/// What refreshing the marks works with.
pub struct Markers<'a> {
    /// The database.
    pub store: &'a Store,
    /// Its path: the tab-label lock lives next to it.
    pub db_path: &'a Path,
    /// herdr.
    pub herdr: &'a mut dyn Surface,
    /// The plugin's `config.toml` (the tab prefix).
    pub config: &'a mut ConfigFile,
    /// How long a label update waits for the lock (Python `LOCK_TIMEOUT`).
    pub lock_wait: Duration,
    /// Where warnings go (Python's `log.warning`).
    pub warn: &'a mut dyn FnMut(&str),
}

impl Markers<'_> {
    /// Refreshes `pane`'s marks (nothing for no pane). Markers are cosmetic:
    /// a failure never fails the change that asked for them; it is warned
    /// about, as Python logs it.
    pub fn update(&mut self, pane: Option<&str>) {
        let Some(pane) = pane.filter(|p| !p.is_empty()) else {
            return;
        };
        if let Err(err) = self.update_now(pane) {
            (self.warn)(&format!(
                "could not refresh the inbox markers for {pane}: {err}"
            ));
        }
    }

    fn update_now(&mut self, pane: &str) -> Result<(), Box<dyn std::error::Error>> {
        let count = self.store.open_counts()?.get(pane).copied().unwrap_or(0);
        self.herdr.set_open_count(pane, count);
        let settings = settings::read(self.config, self.warn);
        if settings.prefix.is_empty() {
            return Ok(()); // disabled: stale prefixes go at the next sync
        }
        let pane_tabs = self.herdr.pane_tabs().unwrap_or_default();
        let tab = pane_tabs
            .iter()
            .rev()
            .find(|(p, _)| p == pane)
            .and_then(|(_, t)| t.clone())
            .filter(|t| !t.is_empty());
        let Some(tab) = tab else {
            return Ok(());
        };
        let Some(_held) = marker_lock(self.db_path, self.lock_wait, self.warn)? else {
            return Ok(());
        };
        let count = tab_counts(&self.store.open_counts()?, &pane_tabs)
            .get(&tab)
            .copied()
            .unwrap_or(0);
        // the settings are read again when needed, as Python's `desired_label` does
        let (config, warn) = (&mut *self.config, &mut *self.warn);
        let unnamed = || settings::read(config, warn).prefix_unnamed;
        apply(
            self.store.db(),
            self.herdr,
            &tab,
            count,
            &settings.prefix,
            unnamed,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
