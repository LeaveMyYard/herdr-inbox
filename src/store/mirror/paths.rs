//! The remote's uv, herdr and plugin root, probed once and cached in
//! `remote_machines` (Python `remote._paths`, `probe`): a puller re-probes
//! over ssh only while one is missing. The two ssh calls are passed in.

use rusqlite::params;
use serde_json::Value;

use super::Machine;
use crate::store::Store;

/// The plugin's id in `herdr plugin list` (Python `PLUGIN_ID`).
pub const PLUGIN_ID: &str = "leavemyyard.herdr-inbox";

/// Python `SAFE_PATH`: `/[A-Za-z0-9._/+-]{1,500}`, the whole line.
pub fn safe_path(s: &str) -> bool {
    let rest = match s.strip_prefix('/') {
        Some(r) => r,
        None => return false,
    };
    (1..=500).contains(&rest.len())
        && rest
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'/' | b'+' | b'-'))
}

/// Python `probe`'s parse of the probe's stdout: `{basename: path}` for each
/// stripped line that is a safe absolute path (a later one wins).
pub fn probed(stdout: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in python_lines(stdout) {
        let line = line.trim_matches(crate::text::py_isspace);
        if safe_path(line) {
            let name = line.rsplit('/').next().unwrap_or("").to_string();
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = line.to_string(),
                None => out.push((name, line.to_string())),
            }
        }
    }
    out
}

/// `str.splitlines()`.
fn python_lines(text: &str) -> Vec<&str> {
    let is_break = |c: char| {
        matches!(
            c,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        )
    };
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.char_indices().find(|(_, c)| is_break(*c)) {
            Some((i, c)) => {
                out.push(&rest[..i]);
                let skip = if c == '\r' && rest[i + 1..].starts_with('\n') {
                    2
                } else {
                    c.len_utf8()
                };
                rest = &rest[i + skip..];
            }
            None => {
                out.push(rest);
                break;
            }
        }
    }
    out
}

/// Python's type name of a JSON value (for its AttributeError/TypeError texts).
fn py_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// `x.get(k, default)` on a JSON value as Python runs it: an error unless `x` is a dict.
fn get<'a>(x: &'a Value, k: &str) -> Result<Option<&'a Value>, String> {
    match x {
        Value::Object(m) => Ok(m.get(k)),
        other => Err(format!(
            "'{}' object has no attribute 'get'",
            py_type(other)
        )),
    }
}

/// The inbox's plugin root in `herdr plugin list --json`'s stdout, as
/// Python reads it (`json.loads(stdout or "{}").get("result", {}).get("plugins", [])`).
fn plugin_root(stdout: &str) -> Result<Option<Value>, String> {
    let text = if stdout.is_empty() { "{}" } else { stdout };
    let Ok(data) = crate::pyjson::loads(text) else {
        return Ok(None); // ValueError: no plugins
    };
    let empty_map = Value::Object(Default::default());
    let result = get(&data, "result")?.unwrap_or(&empty_map);
    let empty_list = Value::Array(Vec::new());
    let plugins = get(result, "plugins")?.unwrap_or(&empty_list);
    let items: Vec<Value> = match plugins {
        Value::Array(list) => list.clone(),
        Value::String(s) => s.chars().map(|c| Value::String(c.to_string())).collect(),
        Value::Object(m) => m.keys().map(|k| Value::String(k.clone())).collect(),
        other => return Err(format!("'{}' object is not iterable", py_type(other))),
    };
    for p in &items {
        if get(p, "plugin_id")?.and_then(Value::as_str) == Some(PLUGIN_ID) {
            return Ok(get(p, "plugin_root")?.cloned());
        }
    }
    Ok(None)
}

impl Store {
    /// The remote's paths (Python `_paths`): `rec` as it is when all three
    /// are cached; else `probe(target)` (the probe's stdout) and
    /// `plugin_list(target, herdr)` (`herdr plugin list --json`'s stdout),
    /// validated and written to `remote_machines`. `Err` is the pull's
    /// failure text.
    pub fn machine_paths(
        &self,
        rec: &Machine,
        probe: impl FnOnce(&str) -> String,
        plugin_list: impl FnOnce(&str, &str) -> String,
    ) -> Result<Result<Machine, String>, rusqlite::Error> {
        let have = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.is_empty());
        if have(&rec.uv) && have(&rec.herdr) && have(&rec.plugin_root) {
            return Ok(Ok(rec.clone()));
        }
        let target = rec.target.clone().unwrap_or_default();
        let bins = probed(&probe(&target));
        let bin = |n: &str| {
            bins.iter()
                .find(|(name, _)| name == n)
                .map(|(_, p)| p.clone())
        };
        let (Some(uv), Some(herdr)) = (bin("uv"), bin("herdr")) else {
            return Ok(Err("uv/herdr not found on the remote".into()));
        };
        let root = match plugin_root(&plugin_list(&target, &herdr)) {
            Ok(Some(Value::String(r))) if safe_path(&r) => r,
            Ok(_) => {
                return Ok(Err(
                    "herdr-inbox is not installed there (inbox remote install)".into(),
                ));
            }
            Err(e) => return Ok(Err(e)),
        };
        self.db().execute(
            "UPDATE remote_machines SET uv = ?, herdr = ?, plugin_root = ? WHERE machine_id = ?",
            params![uv, herdr, root, rec.machine_id],
        )?;
        Ok(Ok(Machine {
            uv: Some(uv),
            herdr: Some(herdr),
            plugin_root: Some(root),
            ..rec.clone()
        }))
    }
}
