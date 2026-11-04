//! [`Surface`] over the kit's herdr client: the calls Python's `herdr.py`
//! makes through the CLI, made over the socket. A failed call is warned
//! about (`herdr <method> failed: <error>`, as Python logs it) and reads as
//! nothing.

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{HerdrError, PaneId, Sanitized, TabId};

use super::{SOURCE, Surface, TOKEN};

/// The live herdr.
pub struct Live {
    herdr: Herdr,
    warn: fn(&str),
}

impl Live {
    /// Calls `herdr`; failures go to `warn`.
    pub fn new(herdr: Herdr, warn: fn(&str)) -> Live {
        Live { herdr, warn }
    }

    fn ok<T>(&self, method: &str, result: Result<T, HerdrError>) -> Result<T, String> {
        result.map_err(|e| {
            let err = e.to_string();
            (self.warn)(&format!("herdr {method} failed: {err}"));
            err
        })
    }
}

impl Surface for Live {
    fn set_open_count(&mut self, pane: &str, count: i64) {
        let value = (count > 0).then(|| Sanitized::new(&format!("?{count}")));
        let sent = self
            .herdr
            .set_token(&PaneId::new(pane), SOURCE, TOKEN, value.as_ref());
        let _ = self.ok("pane.report_metadata", sent);
    }

    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        let panes = self.ok("pane.list", self.herdr.panes()).ok()?;
        Some(
            panes
                .into_iter()
                .filter_map(|p| {
                    let pane = p.pane_id?.as_str().to_string();
                    (!pane.is_empty()).then(|| (pane, p.tab_id.map(|t| t.as_str().to_string())))
                })
                .collect(),
        )
    }

    fn tab_label(&mut self, tab: &str) -> Option<String> {
        let got = self.herdr.tab(&TabId::new(tab));
        self.ok("tab.get", got).ok()?.label
    }

    fn rename_tab(&mut self, tab: &str, label: &str) -> Result<(), String> {
        let got = self
            .herdr
            .rename_tab(&TabId::new(tab), &Sanitized::new(label));
        self.ok("tab.rename", got).map(drop)
    }

    fn notify(&mut self, title: &str, body: &str) {
        let sent = self.herdr.notify(
            &Sanitized::new(title),
            &Sanitized::new(body),
            Some("request"),
        );
        let _ = self.ok("notification.show", sent);
    }
}
