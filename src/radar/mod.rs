//! `$inbox_q` in herdr-radar's sidebar rows (Python `radar.py`).
//!
//! herdr-radar replaces herdr's agent rows with its own, written between
//! marker comments ([`START`], [`END`]) in herdr's `config.toml`; those rows
//! have no slot for another plugin's token, so the inbox's `$inbox_q` (the
//! ❓ count) never shows. `inbox radar-sidebar` adds our cell to every agent
//! row holding radar's `$title_*` tokens, right after `$title_unknown`, on
//! demand only:
//!
//! - only inside radar's markers, in `rows` of `[ui.sidebar.agents]` and every
//!   entry of `[ui.sidebar.agents.rows_by_agent]`; the rest byte for byte
//!   ([`rows`], [`edit`]);
//! - idempotent; never past herdr's 16-cell row limit;
//! - the file must parse before and after; written atomically with a backup,
//!   only if nobody changed it meanwhile ([`file`](mod@file)); herdr reloads its config
//!   only after a change;
//! - it steps aside once radar's own config carries `$inbox_q`
//!   (`extra_tokens`, hhdebb/herdr-radar#57);
//! - `--remove` takes our cells out again.
//!
//! [`missing`] is the popup's read-only check for its hint.

pub mod edit;
pub mod file;
pub mod rows;
pub mod status;

use std::path::{Path, PathBuf};

use herdr_agent_kit::plugin_env::PluginEnv;
use herdr_agent_kit::plugin_env::config::ConfigFile;
use serde_json::{Map, Value};

use crate::surface::Surface;
use crate::token::hex8;
use edit::{edited, unsupported};
pub use status::Status;

/// herdr-radar's plugin id.
pub const RADAR_ID: &str = "hhdebb.herdr-radar";
/// The block's first line.
pub const START: &str = "# >>> herdr-radar sidebar block";
/// The block's last line.
pub const END: &str = "# <<< herdr-radar sidebar block";
/// The token our cell shows.
pub const TOKEN: &str = "$inbox_q";
/// herdr refuses the whole file for one row with more cells (radar#40).
pub const ROW_TOKEN_LIMIT: usize = 16;
/// Our cell's colour when the row has no `$title_blocked` (radar's red).
pub const FALLBACK_FG: &str = "#c04a4a";

/// herdr's `config.toml`, as radar finds it: `$HERDR_INBOX_HERDR_CONFIG`
/// (tests), else `${XDG_CONFIG_HOME:-~/.config}/herdr/config.toml`.
pub fn config_path(env: &PluginEnv) -> PathBuf {
    match env.path_var("HERDR_INBOX_HERDR_CONFIG") {
        Some(p) => p.to_path_buf(),
        None => env.xdg_config_home().join("herdr").join("config.toml"),
    }
}

/// `text` parses as TOML 1.0 (Python `tomllib.loads`), through the kit's
/// config reader (it reads files only, so `text` goes to a private
/// temporary directory, removed after).
pub fn parses(text: &str) -> bool {
    let dir = std::env::temp_dir().join(format!(
        "herdr-inbox-toml-{}-{}",
        std::process::id(),
        hex8()
    ));
    if std::fs::create_dir(&dir).is_err() {
        return false;
    }
    let ok = std::fs::write(dir.join("config.toml"), text).is_ok() && {
        let mut config = ConfigFile::in_dir(&dir);
        config.load_as::<Map<String, Value>>().is_ok() && config.error().is_none()
    };
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

/// Radar carries `$inbox_q` itself: its own `config.toml` (in `radar_dir`)
/// has an `extra_tokens` entry naming it, at any depth. A mention elsewhere,
/// our own cells or an empty list do not count.
pub fn stepped_aside(radar_dir: Option<&Path>) -> bool {
    let Some(dir) = radar_dir else { return false };
    let mut config = ConfigFile::in_dir(dir);
    let table: Map<String, Value> = config.load_as().unwrap_or_default();
    config.error().is_none() && extra_tokens(&table)
}

fn extra_tokens(table: &Map<String, Value>) -> bool {
    table.iter().any(|(k, v)| match v {
        _ if k == "extra_tokens" => carries_inbox_q(v),
        Value::Object(m) => extra_tokens(m),
        _ => false,
    })
}

fn carries_inbox_q(v: &Value) -> bool {
    match v {
        Value::String(s) => s.trim_start_matches('$') == "inbox_q",
        Value::Object(m) => m.get("token").is_some_and(carries_inbox_q),
        Value::Array(a) => a.iter().any(carries_inbox_q),
        _ => false,
    }
}

/// Puts `$inbox_q` into radar's agent rows in `path` (or takes it out,
/// `remove`), as Python `radar.ensure` does, check for check.
pub fn ensure(
    path: &Path,
    remove: bool,
    herdr: &mut dyn Surface,
    warn: &mut dyn FnMut(&str),
) -> Status {
    let Some(text) = file::read(path) else {
        return Status::NoConfig;
    };
    if !herdr.radar_installed() {
        return Status::NoRadar;
    }
    let (starts, ends) = (text.matches(START).count(), text.matches(END).count());
    if starts != 1 || ends != 1 || text.find(END) < text.find(START) {
        return Status::NoBlock;
    }
    if !parses(&text) {
        return Status::Invalid;
    }
    if unsupported(&text) {
        return Status::Unsupported;
    }
    if !remove && stepped_aside(herdr.radar_config_dir().as_deref()) {
        return Status::SteppedAside;
    }
    let Some(new) = edited(&text, remove, &parses, path, warn) else {
        return Status::Unchanged;
    };
    if !file::write(path, &text, &new) {
        return Status::Conflict;
    }
    herdr.reload_config();
    Status::Changed
}

/// Read-only, for the popup's hint: radar is installed and its rows in
/// `path` lack `$inbox_q`. `hint` is the inbox's `radar_sidebar_hint`
/// setting. One file read; herdr is asked only when the rows lack it.
pub fn missing(
    path: &Path,
    hint: bool,
    herdr: &mut dyn Surface,
    warn: &mut dyn FnMut(&str),
) -> bool {
    if !hint {
        return false;
    }
    let Some(text) = file::read(path) else {
        return false;
    };
    if edited(&text, false, &parses, path, warn).is_none() {
        return false;
    }
    !stepped_aside(herdr.radar_config_dir().as_deref()) && herdr.radar_installed()
}

#[cfg(test)]
mod tests;
