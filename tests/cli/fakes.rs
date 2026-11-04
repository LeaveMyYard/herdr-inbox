//! herdr, the delivery and the worker as the CLI golden stubs them
//! (tools/capture_cli.py): every call recorded as an effect, herdr's
//! answers from the step.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use herdr_inbox::clock::now;
use herdr_inbox::service::Delivery;
use herdr_inbox::store::{Question, Store};
use herdr_inbox::surface::Surface;
use serde_json::{Value, json};

/// What a step's herdr and delivery do.
#[derive(Default)]
pub struct World {
    pub pane_tabs: Option<Vec<(String, Option<String>)>>,
    pub labels: BTreeMap<String, String>,
    pub rename_fails: bool,
    /// herdr's error opening the popup.
    pub open_error: Option<String>,
    /// herdr-radar is installed.
    pub radar: bool,
    /// Every tab herdr lists (their labels from `labels`), or unreachable.
    pub all_tabs: Option<Vec<String>>,
    /// herdr-radar's config dir (set by the test).
    pub radar_dir: Option<std::path::PathBuf>,
    /// The stubbed delivery's outcome: delivered, else this error.
    pub delivered: bool,
    pub error: Option<String>,
}

impl World {
    pub fn from_step(a: &Value) -> World {
        let pane_tabs = a.get("pane_tabs").and_then(Value::as_array).map(|list| {
            list.iter()
                .map(|p| {
                    (
                        p[0].as_str().unwrap().to_string(),
                        p[1].as_str().map(str::to_string),
                    )
                })
                .collect()
        });
        let labels = a
            .get("labels")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let d = a.get("delivery").cloned().unwrap_or(Value::Null);
        World {
            pane_tabs,
            labels,
            rename_fails: a.get("rename_fails").and_then(Value::as_bool) == Some(true),
            open_error: a
                .get("open_error")
                .and_then(Value::as_str)
                .map(str::to_string),
            radar: a.get("radar").and_then(Value::as_bool) == Some(true),
            radar_dir: None,
            all_tabs: a
                .get("all_tabs")
                .and_then(Value::as_array)
                .map(|l| l.iter().map(|t| t.as_str().unwrap().to_string()).collect()),
            delivered: d.get("delivered").and_then(Value::as_bool) == Some(true),
            error: d.get("error").and_then(Value::as_str).map(str::to_string),
        }
    }
}

pub type Effects = Rc<RefCell<Vec<Value>>>;

/// The stubbed herdr.
pub struct FakeHerdr {
    pub world: Rc<RefCell<World>>,
    pub effects: Effects,
}

impl Surface for FakeHerdr {
    fn set_open_count(&mut self, pane: &str, count: i64) {
        self.effects
            .borrow_mut()
            .push(json!(["token", pane, count]));
    }

    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        self.effects.borrow_mut().push(json!(["pane_tabs"]));
        self.world.borrow().pane_tabs.clone()
    }

    fn tab_label(&mut self, tab: &str) -> Option<String> {
        self.effects.borrow_mut().push(json!(["tab_label", tab]));
        self.world.borrow().labels.get(tab).cloned()
    }

    fn rename_tab(&mut self, tab: &str, label: &str) -> Result<(), String> {
        self.effects
            .borrow_mut()
            .push(json!(["rename_tab", tab, label]));
        let mut w = self.world.borrow_mut();
        if w.rename_fails {
            return Err("refused".into());
        }
        w.labels.insert(tab.to_string(), label.to_string());
        Ok(())
    }

    fn notify(&mut self, title: &str, body: &str) {
        self.effects
            .borrow_mut()
            .push(json!(["notify", title, body]));
    }

    fn open_popup(&mut self, env: &[(String, String)]) -> Result<(), String> {
        let env: serde_json::Map<String, Value> = env
            .iter()
            .map(|(k, v)| {
                let v = if k == "INBOX_OPEN_NONCE" {
                    "<nonce>"
                } else {
                    v
                };
                (k.clone(), json!(v))
            })
            .collect();
        self.effects.borrow_mut().push(json!(["open_popup", env]));
        match &self.world.borrow().open_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn radar_installed(&mut self) -> bool {
        self.effects.borrow_mut().push(json!(["radar_installed"]));
        self.world.borrow().radar
    }

    fn radar_config_dir(&mut self) -> Option<std::path::PathBuf> {
        self.effects.borrow_mut().push(json!(["radar_config_dir"]));
        self.world.borrow().radar_dir.clone()
    }

    fn all_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        self.effects.borrow_mut().push(json!(["all_tabs"]));
        let w = self.world.borrow();
        w.all_tabs.as_ref().map(|tabs| {
            tabs.iter()
                .map(|t| (t.clone(), w.labels.get(t).cloned()))
                .collect()
        })
    }

    fn reload_config(&mut self) {
        self.effects.borrow_mut().push(json!(["reload_config"]));
    }
}

/// The stubbed delivery and worker.
pub struct FakeDelivery {
    pub world: Rc<RefCell<World>>,
    pub effects: Effects,
}

impl Delivery for FakeDelivery {
    fn ensure_worker(&mut self, server: Option<&str>) {
        self.effects.borrow_mut().push(json!(["ensure", server]));
    }

    fn deliver(&mut self, store: &Store, qs: &[Question]) -> rusqlite::Result<()> {
        let ids: Vec<i64> = qs.iter().map(|q| q.id).collect();
        self.effects
            .borrow_mut()
            .push(json!(["deliver", ids, false, false]));
        let w = self.world.borrow();
        for q in qs {
            if w.delivered {
                store.db().execute(
                    "UPDATE questions SET delivered_at = ?, delivery_error = NULL WHERE id = ?",
                    rusqlite::params![now(), q.id],
                )?;
            } else {
                store.db().execute(
                    "UPDATE questions SET delivery_error = ? WHERE id = ?",
                    rusqlite::params![w.error, q.id],
                )?;
            }
        }
        Ok(())
    }
}
