//! [`Surface`] over the kit's herdr client: the calls Python's `herdr.py`
//! makes through the CLI, made over the socket. A failed call is warned
//! about (`herdr <method> failed: <error>`, as Python logs it) and reads as
//! nothing.

use herdr_agent_kit::herdr_api::blocking::Herdr;
use std::path::PathBuf;

use herdr_agent_kit::herdr_api::{HerdrError, OpenPluginPane, PaneId, Sanitized, TabId};

use serde_json::{Value, json};

use super::popup::{ENTRYPOINT, FOCUS_ENV, INBOX_PLUGIN_ID};
use super::{SOURCE, Surface, TOKEN};
use crate::radar::RADAR_ID;
use crate::remote::pyvalue::py_truthy;

/// The live herdr.
pub struct Live {
    herdr: Herdr,
    warn: fn(&str),
    root: Option<PathBuf>,
    focused: Option<String>,
}

impl Live {
    /// Calls `herdr`; failures go to `warn`.
    pub fn new(herdr: Herdr, warn: fn(&str)) -> Live {
        Live {
            herdr,
            warn,
            root: None,
            focused: None,
        }
    }

    /// Where the popup opens from: the inbox's own checkout (`None`: ask
    /// herdr where it is installed) and the pane that was focused.
    pub fn with_popup(mut self, root: Option<PathBuf>, focused: Option<String>) -> Live {
        self.root = root;
        self.focused = focused;
        self
    }

    /// The inbox's root as herdr lists it (Python `herdr._listed_root`).
    fn listed_root(&self) -> Option<PathBuf> {
        let plugins = self.ok("plugin.list", self.herdr.plugins()).ok()?;
        plugins
            .into_iter()
            .find(|p| p.plugin_id.as_deref() == Some(INBOX_PLUGIN_ID))
            .and_then(|p| p.plugin_root)
            .filter(|r| !r.is_empty())
            .map(PathBuf::from)
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

    fn all_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        let spaces = self.ok("workspace.list", self.herdr.workspaces()).ok()?;
        let mut out = Vec::new();
        for ws in spaces.into_iter().filter_map(|w| w.workspace_id) {
            let tabs = self
                .ok("tab.list", self.herdr.tabs(&ws))
                .unwrap_or_default();
            out.extend(
                tabs.into_iter()
                    .filter_map(|t| Some((t.tab_id?.as_str().to_string(), t.label))),
            );
        }
        Some(out)
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

    fn open_popup(&mut self, env: &[(String, String)]) -> Result<(), String> {
        let mut open = OpenPluginPane::new(INBOX_PLUGIN_ID, ENTRYPOINT);
        open.cwd = self.root.clone().or_else(|| self.listed_root());
        open.env = env.to_vec();
        if let Some(pane) = &self.focused {
            open.env.push((FOCUS_ENV.to_string(), pane.clone()));
        }
        let opened = self.herdr.open_plugin_pane(&open);
        self.ok("plugin.pane_open", opened).map(drop)
    }

    fn radar_installed(&mut self) -> bool {
        let Ok(listed) = self.ok("plugin.list", self.herdr.raw("plugin.list", json!({}))) else {
            return false;
        };
        let plugins = listed.get("plugins").and_then(Value::as_array);
        plugins.into_iter().flatten().any(|p| {
            p.get("plugin_id").and_then(Value::as_str) == Some(RADAR_ID)
                && p.get("enabled").is_none_or(py_truthy)
        })
    }

    fn radar_config_dir(&mut self) -> Option<PathBuf> {
        let dir = self.herdr.plugin_config_dir(RADAR_ID);
        self.ok("plugin config-dir", dir).ok().flatten()
    }

    fn reload_config(&mut self) {
        let reloaded = self.herdr.reload_config();
        let _ = self.ok("server.reload_config", reloaded);
    }
}
