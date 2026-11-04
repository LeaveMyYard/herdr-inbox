//! Whether a text is TOML 1.0, as Python's `tomllib.loads` reads it: the
//! kit's config reader decides. It reads files only, so the text goes to a
//! private temporary directory (`0700`, removed after).

use std::io;

use herdr_agent_kit::plugin_env::config::ConfigFile;
use serde_json::{Map, Value};

/// `text` parses as TOML 1.0. `Err`: the check could not be made (no
/// temporary file), which says nothing about the text.
pub fn parses(text: &str) -> io::Result<bool> {
    let dir = tempfile::Builder::new()
        .prefix("herdr-inbox-toml-")
        .tempdir()?;
    std::fs::write(dir.path().join("config.toml"), text)?;
    let mut config = ConfigFile::in_dir(dir.path());
    Ok(config.load_as::<Map<String, Value>>().is_ok() && config.error().is_none())
}
