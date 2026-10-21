//! The typing half of a pane's pass (Python `deliver_batch`'s message loop):
//! the messages, the input box, the prompt with the transcripts watched
//! around it, and where each typed answer landed. A batch's wait for its
//! text happens once, when its first landing is asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use herdr_agent_kit::herdr_api::clean;
use inbox_core::deliver::{COMPOSER_DRAFT, COMPOSER_UNKNOWN, Composer};
use inbox_core::verify::Landing;
use serde_json::{Value, json};

use super::look::PaneLook;
use crate::delivery::messages::delivery_messages;
use crate::delivery::pane::{composer, prompt};
use crate::delivery::plans::{Plan, verify_plans, watch_paths};
use crate::delivery::segments::answered;
use crate::delivery::watch::{Watched, size, wait_hits};

/// No cap (Python `herdr.clean(text, limit=10**9)`).
const NO_LIMIT: usize = 1_000_000_000;

/// One message of the pass, and once typed, where it may land.
#[derive(Debug, Default)]
pub struct Typed {
    ids: Vec<i64>,
    text: String,
    full: bool,
    typed: bool,
    pub(super) path: Option<PathBuf>,
    offset: u64,
    watch: Vec<Watched>,
    plans: Option<BTreeMap<i64, Plan>>,
    hits: Option<BTreeSet<PathBuf>>,
}

impl PaneLook<'_, '_> {
    /// Python `delivery_messages` over `ids`, remembered for typing.
    pub(super) fn batches(&mut self, ids: &[i64]) -> Vec<(Vec<i64>, bool)> {
        let answers: Vec<_> = ids.iter().map(|id| answered(self.q(*id))).collect();
        let messages = delivery_messages(&answers);
        let mut out = Vec::new();
        for m in messages {
            let ids: Vec<i64> = m
                .ids
                .iter()
                .map(|i| i64::try_from(*i).unwrap_or(0))
                .collect();
            out.push((ids.clone(), m.full));
            self.typed.push(Typed {
                ids,
                text: m.text,
                full: m.full,
                ..Typed::default()
            });
        }
        out
    }

    pub(super) fn typed_of(&self, ids: &[i64]) -> Option<&Typed> {
        self.typed.iter().find(|t| t.ids == ids)
    }

    fn refs(&self, ids: &[i64]) -> String {
        ids.iter()
            .map(|id| format!("Q{id}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Claude Code's input box, with the two notices that are the caller's:
    /// once per pane for a box that stays unreadable, and under "Send now"
    /// that the answers were not sent.
    pub(super) fn composer_state(&mut self, ids: &[i64], now: Instant) -> Composer {
        let state = composer(self.ctx.herdr, &self.pane);
        let refs = self.refs(ids);
        let first = ids.first().map(|id| self.q(*id));
        let where_ = first
            .and_then(|q| q.tab_label.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or(self.pane.clone());
        if let Some((title, body)) = self
            .ctx
            .unreadable
            .note(&self.pane, state, &where_, &refs, now)
        {
            (self.ctx.notify)(&title, &body);
        }
        if state != Composer::Empty && self.mode.force {
            let reason = if state == Composer::Draft {
                COMPOSER_DRAFT
            } else {
                COMPOSER_UNKNOWN
            };
            (self.ctx.notify)(
                &format!("{refs} not sent"),
                reason.trim_start_matches("waiting: "),
            );
        }
        state
    }

    /// Types the message for `ids`: herdr's error, if any. The transcripts
    /// to watch are the followed and reported sessions', plus the one the
    /// prompt itself may have created.
    pub(super) fn type_batch(&mut self, ids: &[i64]) -> Option<String> {
        let agent = self.agent.clone()?;
        let watch = watch_paths(self.ctx.store.db(), self.ctx.files, &agent, false);
        let mut watch = self.ok(watch);
        let mut path = self.path.clone();
        let mut offset = path.as_deref().map_or(0, size);
        let text = self.typed_of(ids)?.text.clone();
        if let Some(err) = prompt(self.ctx.herdr, &self.pane, &text) {
            return Some(err);
        }
        if path.is_none() {
            // a brand-new session's transcript may appear with this very prompt
            path = self.find(agent.session.as_deref());
            offset = 0;
        }
        if let Some(p) = &path
            && !watch.iter().any(|(w, _)| w == p)
        {
            watch.push((p.clone(), offset));
        }
        if path.is_none() {
            let fresh = watch_paths(self.ctx.store.db(), self.ctx.files, &agent, true);
            watch = self.ok(fresh);
            path = watch.first().map(|(p, _)| p.clone());
        }
        self.path = path.clone();
        if let Some(t) = self.typed.iter_mut().find(|t| t.ids == ids) {
            (t.typed, t.path, t.offset, t.watch, t.text) =
                (true, path, offset, watch, clean(&text, NO_LIMIT));
        }
        None
    }

    fn plans_of(&mut self, k: usize) -> BTreeMap<i64, Plan> {
        if let Some(p) = &self.typed[k].plans {
            return p.clone();
        }
        let batch: Vec<_> = self.typed[k].ids.iter().map(|id| self.q(*id)).collect();
        let plans = verify_plans(
            self.ctx.store.db(),
            self.ctx.files,
            &self.pane,
            &self.typed[k].watch,
            &batch,
        );
        let plans = self.ok(plans);
        self.typed[k].plans = Some(plans.clone());
        plans
    }

    /// Where answer `id`'s text landed, by its own plan.
    pub(super) fn landing_of(&mut self, id: i64) -> Landing {
        let Some(k) = self
            .typed
            .iter()
            .position(|t| t.typed && t.ids.contains(&id))
        else {
            return Landing::default();
        };
        let plans = self.plans_of(k);
        if self.typed[k].hits.is_none() {
            let t = &self.typed[k];
            let qids: Vec<u64> = t
                .ids
                .iter()
                .map(|i| u64::try_from(*i).unwrap_or(0))
                .collect();
            let hits = wait_hits(&t.watch, &qids, Some(&t.text), self.ctx.verify_wait);
            self.typed[k].hits = Some(hits);
        }
        let hits = self.typed[k].hits.clone().unwrap_or_default();
        plans
            .get(&id)
            .map_or_else(Landing::default, |p| p.landing(&hits))
    }

    /// The verify-later jobs of the typed messages with a transcript (Python
    /// `verify_later({...})`, read by `inbox verify`).
    pub(super) fn jobs(&mut self) -> Vec<Value> {
        let busy = self
            .agent
            .as_ref()
            .is_some_and(|a| !matches!(a.status.as_str(), "idle" | "done"));
        let mut jobs = Vec::new();
        for k in 0..self.typed.len() {
            // only a typed message has a path
            let Some(path) = self.typed[k].path.clone() else {
                continue;
            };
            let plans = self.plans_of(k);
            let t = &self.typed[k];
            let rows: Vec<Value> = t
                .ids
                .iter()
                .map(|id| {
                    let plan = plans.get(id).cloned().unwrap_or_default();
                    let counting: Vec<String> = plan
                        .counting
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect();
                    json!({"id": id, "claim": self.claims.get(id).map(|c| c.0.clone()),
                           "counting": counting, "older_error": plan.older_error})
                })
                .collect();
            let watch: Vec<Value> = t
                .watch
                .iter()
                .map(|(p, off)| json!([p.display().to_string(), off]))
                .collect();
            jobs.push(json!({"path": path.display().to_string(), "offset": t.offset, "ack": t.full, "busy": busy,
                             "watch": watch, "text": t.text, "rows": rows}));
        }
        jobs
    }
}
