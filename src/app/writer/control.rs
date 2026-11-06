//! `control.json`: `{"v":1,"disabled":false}`. The installer writes it. Absent
//! means no install to serve. Disabled, no writer runs or starts.

use serde::{Deserialize, Serialize};

use super::files::Paths;

const NAME: &str = "control.json";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Control {
    v: u8,
    disabled: bool,
}

/// `Ok` when a writer may run here; why not otherwise. A file this cannot read
/// says no: only an install clears it.
pub fn enabled(paths: &Paths) -> Result<(), String> {
    let bytes = std::fs::read(paths.file(NAME)).map_err(|e| format!("no install here: {e}"))?;
    let control: Control =
        serde_json::from_slice(&bytes).map_err(|e| format!("control.json: {e}"))?;
    if control.v != 1 {
        return Err("control.json is another version".to_string());
    }
    if control.disabled {
        return Err("the feed writer is disabled".to_string());
    }
    Ok(())
}

/// Sets `disabled`, which only a reinstall clears.
pub fn disable(paths: &Paths) -> Result<(), String> {
    let control = Control {
        v: 1,
        disabled: true,
    };
    let bytes = serde_json::to_vec(&control).map_err(|e| e.to_string())?;
    paths
        .replace(NAME, ".herdr-inbox-control.tmp", &bytes)
        .map_err(|e| format!("could not disable the writer: {e}"))
}
