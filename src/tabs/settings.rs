//! The tab-prefix settings (Python `tabs.settings`, `prefix`,
//! `prefix_unnamed`): `config.toml` in the inbox's herdr plugin config dir,
//! the one place herdr-side processes and Claude-spawned ones both see:
//!
//! ```toml
//! tab_prefix = "❓"            # "" turns the prefix off
//! prefix_unnamed_tabs = false
//! ```
//!
//! Read on every use (a [`ConfigFile`] re-parses only when the file
//! changed), so an edit applies without restarting anything.

use herdr_agent_kit::plugin_env::config::ConfigFile;
use serde_json::{Map, Value};

use super::DEFAULT_PREFIX;
use crate::text::py_isspace;

/// The inbox's herdr plugin id (Python `tabs.PLUGIN_ID`).
pub const PLUGIN_ID: &str = "leavemyyard.herdr-inbox";

/// The two settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The mark in front of the count; empty: no prefix (Python `tabs.prefix()`).
    pub prefix: String,
    /// Prefix tabs whose label is only their position too (Python `tabs.prefix_unnamed()`).
    pub prefix_unnamed: bool,
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
    }
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

/// Python's `bool()` of a TOML value.
fn py_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests;
