//! The pane as a delivery sees it through herdr (Python `herdr.live_agent`,
//! `composer_state`, `panel_focused`, `prompt_agent`): reads and the one
//! write, each a failure-tolerant call as in Python (a herdr that cannot be
//! asked reads as no agent, an unreadable input box, no panel).

use std::path::Path;

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::herdr_api::{PaneId, Sanitized};
use inbox_core::deliver::{Agent, Composer};
use serde_json::{Value, json};

use super::screen::{composer_state, panel_focused};
use crate::claude::{Transcripts, live_session};
use crate::remote::pyvalue::{py_repr_str, py_truthy};

/// The agent in an `agent.get` result (Python `live_agent` after its call),
/// `live` following a reported session to the one that runs now. `None`
/// when there is none. The status is herdr's own text (`"None"` when it
/// sends none, as Python prints it); an empty reported session stays empty.
pub fn agent_from(result: Option<&Value>, live: impl Fn(&str) -> String) -> Option<Agent> {
    let agent = result?.get("agent").filter(|a| py_truthy(a))?.as_object()?;
    // Python `(agent_session or {}).get("value")`: no falsy JSON value has a `value`
    let reported = agent
        .get("agent_session")
        .and_then(|s| s.get("value"))
        .and_then(Value::as_str);
    let session = reported.map(|r| if r.is_empty() { String::new() } else { live(r) });
    Some(Agent {
        kind: agent.get("agent").and_then(Value::as_str).map(String::from),
        status: py_repr_str(agent.get("agent_status").unwrap_or(&Value::Null)),
        focused: agent.get("focused").is_some_and(py_truthy),
        session,
        reported_session: reported.map(String::from),
    })
}

/// The agent in `pane` now (Python `herdr.live_agent`).
pub fn live_agent(
    herdr: &Herdr,
    files: &Transcripts,
    state_dir: &Path,
    pane: &str,
) -> Option<Agent> {
    let result = herdr
        .raw(
            "agent.get",
            json!({"target": Sanitized::new(pane).as_str()}),
        )
        .ok();
    agent_from(result.as_ref(), |reported| {
        live_session(files, state_dir, reported, Some(pane))
    })
}

/// Claude Code's input box in `pane` (Python `herdr.composer_state`): an
/// unreadable pane is [`Composer::Unknown`], never typed into.
pub fn composer(herdr: &Herdr, pane: &str) -> Composer {
    herdr
        .read_pane(&PaneId::new(pane), 30, true)
        .map_or(Composer::Unknown, |screen| composer_state(&screen))
}

/// Claude Code's background-agents panel has the keyboard in `pane`
/// (Python `herdr.panel_focused`); an unreadable pane says no.
pub fn panel(herdr: &Herdr, pane: &str) -> bool {
    herdr
        .read_pane(&PaneId::new(pane), 20, false)
        .is_ok_and(|screen| panel_focused(&screen))
}

/// Types `text` into the agent in `pane` without waiting (Python
/// `herdr.prompt_agent`, cleaned to herdr's argument cap): herdr's error, if
/// any. The error's wording is the socket client's, not the CLI's.
pub fn prompt(herdr: &Herdr, pane: &str, text: &str) -> Option<String> {
    herdr
        .prompt(&PaneId::new(pane), &Sanitized::new(text))
        .err()
        .map(|e| e.to_string())
}
