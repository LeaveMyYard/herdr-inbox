//! Every marker reconciled with the database (Python `service.sync_tokens`,
//! `inbox sync-tokens`, herdr's startup command): herdr drops pane tokens on
//! restart and a crash can leave a stale tab prefix. `inbox_q` on every
//! pane, then the prefix on every tab, one short locked section per tab;
//! `remove_prefixes` strips every prefix (before uninstalling).

use std::error::Error;

use super::markers::Markers;
use crate::tabs::label::{desired_label, marks_for, tab_counts};
use crate::tabs::lock::marker_lock;
use crate::tabs::settings;
use crate::tabs::{DEFAULT_PREFIX, apply::apply, base_label};

impl Markers<'_> {
    /// Reconciles every pane's token and every tab's prefix.
    pub fn sync(&mut self, remove_prefixes: bool) -> Result<(), Box<dyn Error>> {
        let mark = if remove_prefixes {
            String::new()
        } else {
            settings::read(self.config, self.warn).prefix
        };
        let pane_tabs = self.herdr.pane_tabs();
        let counts = self.store.open_counts()?;
        let panes: Vec<String> = match &pane_tabs {
            Some(list) => {
                let mut panes: Vec<String> = Vec::new();
                for (p, _) in list {
                    if !panes.contains(p) {
                        panes.push(p.clone());
                    }
                }
                panes
            }
            None => self.store.known_panes()?,
        };
        for pane in &panes {
            self.herdr
                .set_open_count(pane, counts.get(pane).copied().unwrap_or(0));
        }
        let Some(pane_tabs) = pane_tabs else {
            return Ok(());
        };
        for (tab, label) in self.herdr.all_tabs().unwrap_or_default() {
            let Some(label) = label else { continue };
            let Some(_held) = marker_lock(self.db_path, self.lock_wait, self.warn)? else {
                continue;
            };
            let count = tab_counts(&self.store.open_counts()?, &pane_tabs)
                .get(&tab)
                .copied()
                .unwrap_or(0);
            let marks = marks_for(self.store.db(), Some(&tab))?;
            let marks_ref: Vec<&str> = marks.iter().map(String::as_str).collect();
            let (config, warn) = (&mut *self.config, &mut *self.warn);
            let mut unnamed = || settings::read(config, warn).prefix_unnamed;
            let want = desired_label(&base_label(&label, &marks_ref), count, &mark, &mut unnamed);
            let custom = marks.iter().any(|m| m != DEFAULT_PREFIX);
            if want != label || custom {
                apply(self.store.db(), self.herdr, &tab, count, &mark, unnamed)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
