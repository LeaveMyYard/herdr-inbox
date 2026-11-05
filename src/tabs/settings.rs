//! The inbox's settings (Python `tabs.settings`, `prefix`,
//! `prefix_unnamed`, `radar.hint_enabled`, `remote.settings`): `config.toml` in the inbox's herdr plugin config dir,
//! the one place herdr-side processes and Claude-spawned ones both see:
//!
//! ```toml
//! tab_prefix = "❓"            # "" turns the prefix off
//! prefix_unnamed_tabs = false
//! radar_sidebar_hint = true      # the popup's herdr-radar hint
//! remote_answer_expiry_hours = 12  # how long a queued remote action waits
//! ```
//!
//! Read on every use (a [`ConfigFile`] re-parses only when the file
//! changed), so an edit applies without restarting anything.

use std::time::Duration;

use herdr_agent_kit::plugin_env::config::ConfigFile;
use serde_json::{Map, Value};

use super::DEFAULT_PREFIX;
use crate::remote::pyvalue::py_truthy;
use crate::text::py_isspace;

/// The inbox's herdr plugin id (Python `tabs.PLUGIN_ID`).
pub const PLUGIN_ID: &str = "leavemyyard.herdr-inbox";

/// `remote_answer_expiry_hours` when unset (Python's default).
const DEFAULT_EXPIRY_HOURS: f64 = 12.0;
/// The longest expiry taken (a century): the expiry time must stay a date.
const MAX_EXPIRY_HOURS: f64 = 876_000.0;

/// The settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The mark in front of the count; empty: no prefix (Python `tabs.prefix()`).
    pub prefix: String,
    /// Prefix tabs whose label is only their position too (Python `tabs.prefix_unnamed()`).
    pub prefix_unnamed: bool,
    /// The popup's hint that herdr-radar's rows lack `$inbox_q` (Python
    /// `radar.hint_enabled()`, `radar_sidebar_hint`, on by default).
    pub radar_hint: bool,
    /// How long a queued remote action waits (Python `remote.settings()`'s
    /// `expiry_hours`, `remote_answer_expiry_hours`).
    pub answer_expiry: Duration,
}

/// The settings as they are in `config` now. A file that is missing or does
/// not parse is no settings, as in Python (which logs the second; so does
/// this, through `warn`).
pub fn read(config: &mut ConfigFile, warn: &mut dyn FnMut(&str)) -> Settings {
    let table: Map<String, Value> = config.load_as().unwrap_or_default();
    if let Some(err) = config.error() {
        warn(err);
    }
    from_table(&table)
}

/// The settings in a parsed `config.toml`.
pub fn from_table(table: &Map<String, Value>) -> Settings {
    let prefix = table
        .get("tab_prefix")
        .map_or_else(|| DEFAULT_PREFIX.to_string(), py_str);
    Settings {
        prefix: prefix.trim_matches(py_isspace).to_string(),
        prefix_unnamed: table.get("prefix_unnamed_tabs").is_some_and(py_truthy),
        radar_hint: table.get("radar_sidebar_hint").is_none_or(py_truthy),
        answer_expiry: answer_expiry(table.get("remote_answer_expiry_hours")),
    }
}

/// Python's `float(value or 12)`, hours. Python stops with an exception on a
/// value it cannot use; here that value is the default (also a negative one
/// or one past [`MAX_EXPIRY_HOURS`], which Python would take or overflow on).
fn answer_expiry(value: Option<&Value>) -> Duration {
    let hours = match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim_matches(py_isspace).parse().ok(),
        Some(Value::Bool(true)) => Some(1.0),
        _ => None,
    };
    let hours = hours
        .filter(|h| *h > 0.0 && *h <= MAX_EXPIRY_HOURS)
        .unwrap_or(DEFAULT_EXPIRY_HOURS);
    Duration::from_secs_f64(hours * 3600.0)
}

/// Python's `str()` of a TOML value, for the scalars a setting can hold.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or_default();
            if f.fract() == 0.0 && f.abs() < 1e16 {
                format!("{f:.1}")
            } else {
                f.to_string()
            }
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;
