//! The radar steps of tests/fixtures/cli.json: herdr's config around one of
//! herdr-radar's own blocks (tests/fixtures/radar, as tools/capture_cli.py
//! builds it), radar's own config, and what `radar-sidebar` left.

use std::path::{Path, PathBuf};

use herdr_inbox::radar::edit::edited;
use herdr_inbox::radar::parses;
use serde_json::Value;

use crate::store_golden::s;

const BEFORE: &str = "# my herdr config\n[keys]\nprefix = \"ctrl-b\"\n\n[ui]\nsidebar_width = 40   # keep \"this\" [as is]\n\n";
const AFTER: &str = "\n# trailing comment, kept\n";

/// capture_cli's `radar_text`.
fn text(spec: &Value) -> String {
    let name = s(spec, "block").unwrap();
    let path = format!(
        "{}/tests/fixtures/radar/{name}.block.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut text = format!("{BEFORE}{}{AFTER}", std::fs::read_to_string(path).unwrap());
    if let Some(r) = spec.get("replace").and_then(Value::as_array) {
        text = text.replacen(r[0].as_str().unwrap(), r[1].as_str().unwrap(), 1);
    }
    if spec.get("edited") == Some(&Value::Bool(true)) {
        text = edited(&text, false, &parses, Path::new("x"), &mut |_| {}).unwrap();
    }
    if spec.get("crlf") == Some(&Value::Bool(true)) {
        text = text.replace('\n', "\r\n");
    }
    text
}

/// Writes the step's herdr config and radar's config; radar's config dir if any.
pub fn prepare(dir: &Path, a: &Value) -> Option<PathBuf> {
    let herdr = dir.join("herdr-config/config.toml");
    std::fs::create_dir_all(herdr.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&herdr);
    if let Some(spec) = a.get("herdr_config").filter(|v| !v.is_null()) {
        std::fs::write(&herdr, text(spec)).unwrap();
    }
    let radar = dir.join("radar-config");
    std::fs::create_dir_all(&radar).unwrap();
    let _ = std::fs::remove_file(radar.join("config.toml"));
    let cfg = s(a, "radar_config")?;
    std::fs::write(radar.join("config.toml"), cfg).unwrap();
    Some(radar)
}

/// What `radar-sidebar` left: herdr's config and whether it backed it up
/// (the backup removed, as the capture removes it).
pub fn left(dir: &Path) -> (Value, bool) {
    let herdr = dir.join("herdr-config/config.toml");
    let text = std::fs::read_to_string(&herdr).map_or(Value::Null, Value::String);
    let bak = dir.join("herdr-config/config.toml.inbox-bak");
    let backup = bak.exists();
    let _ = std::fs::remove_file(bak);
    (text, backup)
}
