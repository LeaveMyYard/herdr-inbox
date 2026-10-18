//! Automatic alerts for agents stuck at an approval or question prompt
//! (Python `alerts.scan_blocked`), less its herdr calls: the caller passes
//! what `herdr agent list` returned, the clock and a tab-label lookup, and
//! carries out the returned effects. Nothing is ever typed into such a pane.
//!
//! A pane counts as blocked once herdr reports it `blocked`; after
//! [`BLOCKED_AFTER`] it gets an alert. It leaves only after [`CLEAR_AFTER`]
//! seconds and two scans of another status (herdr's detection can flicker),
//! or at once when its agent is gone from the list. When each prompt was
//! first seen is in `blocked_panes`, so a restart keeps the clock; the
//! hysteresis memory lives in [`BlockedScan`] and may reset with the process.

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter};
use serde_json::Value as Json;

use super::Effect;
use crate::clock::{now, now_minus};
use crate::store::Store;
use crate::tabs::{DEFAULT_PREFIX, base_label};

/// A prompt answered within this never raises an alert (Python `BLOCKED_AFTER`).
pub const BLOCKED_AFTER: Duration = Duration::from_secs(15);
/// Seconds of another status (and two scans) before a pane counts as unblocked.
pub const CLEAR_AFTER: f64 = 10.0;
const BLOCKED_MESSAGE: &str =
    "The agent is at an approval or question prompt and cannot continue until you answer it there.";

/// One agent of `herdr agent list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenAgent {
    /// Its pane.
    pub pane_id: String,
    /// `agent_status` (`blocked`, `working`, ...).
    pub status: Option<String>,
    /// Its tab, as herdr gave it (stored so; an empty one is not looked up).
    pub tab_id: Option<String>,
    /// `agent_session.value`.
    pub session: Option<String>,
    /// `agent` (its kind).
    pub agent: Option<String>,
}

impl SeenAgent {
    /// One entry of the list; `None` for one Python skips (not an object, no pane).
    pub fn from_json(v: &Json) -> Option<SeenAgent> {
        let text = |k: &str| v.get(k).and_then(Json::as_str).map(String::from);
        let pane_id = text("pane_id").filter(|p| !p.is_empty())?;
        Some(SeenAgent {
            pane_id,
            status: text("agent_status"),
            tab_id: text("tab_id"),
            session: v
                .get("agent_session")
                .and_then(|s| s.get("value"))
                .and_then(Json::as_str)
                .map(String::from),
            agent: text("agent"),
        })
    }
}

/// The scan's memory between runs (Python's module-level `_unblocked`).
#[derive(Debug, Default)]
pub struct BlockedScan {
    unblocked: HashMap<(String, String), (f64, u32)>,
}

impl BlockedScan {
    /// One scan of herdr server `server`: `agents` is what `agent list`
    /// returned (`None`: herdr did not answer, nothing changes), `t` a
    /// monotonic clock in seconds. Returns how many agents herdr reports and
    /// the effects to carry out.
    pub fn scan(
        &mut self,
        store: &Store,
        server: &str,
        agents: Option<&[SeenAgent]>,
        t: f64,
        tab_label: impl Fn(&str) -> Option<String>,
    ) -> rusqlite::Result<(usize, Vec<Effect>)> {
        let Some(agents) = agents else {
            return Ok((0, Vec::new()));
        };
        let present: BTreeSet<&str> = agents.iter().map(|a| a.pane_id.as_str()).collect();
        let mut blocked: Vec<(&str, &SeenAgent)> = Vec::new(); // insertion order, last entry wins
        for a in agents
            .iter()
            .filter(|a| a.status.as_deref() == Some("blocked"))
        {
            match blocked.iter_mut().find(|(p, _)| *p == a.pane_id) {
                Some(slot) => slot.1 = a,
                None => blocked.push((&a.pane_id, a)),
            }
        }
        let is_blocked = |p: &str| blocked.iter().any(|(b, _)| *b == p);
        let mut since: Vec<(String, String)> = {
            let mut stmt = store
                .db()
                .prepare("SELECT pane_id, since FROM blocked_panes WHERE herdr_server = ?")?;
            let rows = stmt.query_map([server], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (pane, _) in since.clone() {
            let key = (server.to_string(), pane.clone());
            if is_blocked(&pane) {
                self.unblocked.remove(&key);
                continue;
            }
            let (first, ticks) = self.unblocked.get(&key).copied().unwrap_or((t, 0));
            self.unblocked.insert(key.clone(), (first, ticks + 1));
            if !present.contains(pane.as_str()) || (ticks + 1 >= 2 && t - first >= CLEAR_AFTER) {
                store.db().execute(
                    "DELETE FROM blocked_panes WHERE herdr_server = ? AND pane_id = ?",
                    params![server, pane],
                )?;
                since.retain(|(p, _)| *p != pane);
                self.unblocked.remove(&key);
            }
        }
        for (pane, _) in &blocked {
            if !since.iter().any(|(p, _)| p == pane) {
                let at = now();
                store.db().execute(
                    "INSERT OR IGNORE INTO blocked_panes (herdr_server, pane_id, since) VALUES (?, ?, ?)",
                    params![server, pane, at],
                )?;
                since.push((pane.to_string(), at));
            }
        }
        let waiting: BTreeSet<String> = since.iter().map(|(p, _)| p.clone()).collect();
        let scope = " AND COALESCE(herdr_server, ?) = ?";
        let mut effects = Vec::new();
        let open_auto = open_blocked(store, server, scope)?;
        for (pane, aid) in &open_auto {
            if !pane.as_ref().is_some_and(|p| waiting.contains(p))
                && let Some(alert) = store.close_alert(*aid, "cleared")?
            {
                effects.push(Effect::Markers(alert.pane_id));
            }
        }
        end_dismissed_episodes(store, server, scope, &waiting)?;
        let dismissed = dismissed(store, server, scope)?;
        let threshold = now_minus(BLOCKED_AFTER);
        for (pane, agent) in &blocked {
            let first_seen = since
                .iter()
                .find(|(p, _)| p == pane)
                .map(|(_, s)| s.as_str())
                .unwrap_or("");
            if open_auto.iter().any(|(p, _)| p.as_deref() == Some(*pane))
                || dismissed.contains(*pane)
                || !(is_time(first_seen) && first_seen <= threshold.as_str())
            {
                continue;
            }
            let label = agent
                .tab_id
                .as_deref()
                .filter(|tab| !tab.is_empty())
                .and_then(&tab_label)
                .filter(|l| !l.is_empty());
            let label = label.map(|l| base_label(&l, &[DEFAULT_PREFIX]));
            let place = label
                .clone()
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| pane.to_string());
            if let Some(aid) =
                raise_blocked(store, server, scope, pane, agent, label.as_deref(), &place)?
            {
                effects.push(Effect::Notify {
                    title: format!("A{aid} · {place}"),
                    body: "waiting for you at a prompt".into(),
                });
                effects.push(Effect::Markers(Some(pane.to_string())));
            }
        }
        Ok((agents.len(), effects))
    }
}

/// Exactly `YYYY-MM-DDTHH:MM:SS+00:00`, as the scan writes `since` (Python's
/// `_age` gives 0, never an alert, for what `fromisoformat` rejects).
fn is_time(s: &str) -> bool {
    let shape = "dddd-dd-ddTdd:dd:dd+00:00";
    s.len() == shape.len()
        && s.bytes().zip(shape.bytes()).all(|(c, w)| {
            if w == b'd' {
                c.is_ascii_digit()
            } else {
                c == w
            }
        })
}

/// Open blocked alerts of `server` (or of none recorded): (pane, id), the
/// last row per pane winning as in Python's dict.
fn open_blocked(
    store: &Store,
    server: &str,
    scope: &str,
) -> rusqlite::Result<Vec<(Option<String>, i64)>> {
    let mut stmt = store.db().prepare(&format!(
        "SELECT pane_id, id FROM alerts WHERE status = 'open' AND kind = 'blocked'{scope}"
    ))?;
    let rows: Vec<(Option<String>, i64)> = stmt
        .query_map([server, server], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out: Vec<(Option<String>, i64)> = Vec::new();
    for (pane, id) in rows {
        match out.iter_mut().find(|(p, _)| *p == pane) {
            Some(slot) => slot.1 = id,
            None => out.push((pane, id)),
        }
    }
    Ok(out)
}

/// A dismissed blocked alert holds until its pane is seen not blocked: then
/// `closed_at` ends its episode, so the next prompt alerts again.
fn end_dismissed_episodes(
    store: &Store,
    server: &str,
    scope: &str,
    waiting: &BTreeSet<String>,
) -> rusqlite::Result<()> {
    let not_in = if waiting.is_empty() {
        String::new()
    } else {
        format!(
            " AND pane_id NOT IN ({})",
            vec!["?"; waiting.len()].join(",")
        )
    };
    let mut args: Vec<Value> = vec![
        now().into(),
        server.to_string().into(),
        server.to_string().into(),
    ];
    args.extend(waiting.iter().map(|p| Value::from(p.clone())));
    store.db().execute(
        &format!("UPDATE alerts SET closed_at = ? WHERE kind = 'blocked' AND status = 'acked' AND closed_at IS NULL{scope}{not_in}"),
        params_from_iter(args),
    )?;
    Ok(())
}

fn dismissed(store: &Store, server: &str, scope: &str) -> rusqlite::Result<BTreeSet<String>> {
    let mut stmt = store.db().prepare(&format!(
        "SELECT pane_id FROM alerts WHERE kind = 'blocked' AND status = 'acked' AND closed_at IS NULL{scope}"
    ))?;
    let rows = stmt.query_map([server, server], |r| r.get::<_, Option<String>>(0))?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect())
}

/// Files the blocked alert of `pane` unless one is open already: its id.
fn raise_blocked(
    store: &Store,
    server: &str,
    scope: &str,
    pane: &str,
    agent: &SeenAgent,
    tab_label: Option<&str>,
    place: &str,
) -> rusqlite::Result<Option<i64>> {
    store.immediate(|db| {
        let open: Option<i64> = rusqlite::OptionalExtension::optional(db.query_row(
            &format!("SELECT 1 FROM alerts WHERE status = 'open' AND kind = 'blocked' AND pane_id = ?{scope}"),
            params![pane, server, server],
            |r| r.get(0),
        ))?;
        if open.is_some() {
            return Ok(None);
        }
        db.execute(
            "INSERT INTO alerts (created_at, kind, severity, title, message, pane_id, tab_id, tab_label, session_id, \
             asked_by, herdr_server) VALUES (?, 'blocked', 'warning', ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                now(),
                format!("{place} is waiting for you"),
                BLOCKED_MESSAGE,
                pane,
                agent.tab_id,
                tab_label,
                agent.session,
                agent.agent,
                server
            ],
        )?;
        Ok(Some(db.last_insert_rowid()))
    })
}
