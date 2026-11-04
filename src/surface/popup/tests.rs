use std::path::PathBuf;

use super::*;

#[test]
fn the_checkout_is_the_nearest_folder_with_the_manifest() {
    let d = tempfile::tempdir().unwrap();
    let bin = d.path().join("target/release");
    std::fs::create_dir_all(&bin).unwrap();
    assert_eq!(own_checkout(&bin.join("herdr-inbox")), None);
    std::fs::write(d.path().join("herdr-plugin.toml"), "").unwrap();
    assert_eq!(
        own_checkout(&bin.join("herdr-inbox")),
        Some(d.path().to_path_buf())
    );
}

fn env(vars: &[(&str, &str)]) -> PluginEnv {
    PluginEnv::from_vars(vars.iter().copied(), Some(PathBuf::from("/h")), 1000)
}

#[test]
fn the_focused_pane_is_herdrs_context_then_our_pane() {
    let ctx = r#"{"focused_pane_id": "w1:p7"}"#;
    assert_eq!(
        focused_pane(&env(&[
            ("HERDR_PLUGIN_CONTEXT_JSON", ctx),
            ("HERDR_PANE_ID", "w1:p2")
        ])),
        Some("w1:p7".into())
    );
    assert_eq!(
        focused_pane(&env(&[
            ("HERDR_PLUGIN_CONTEXT_JSON", "[1]"),
            ("HERDR_PANE_ID", "w1:p2")
        ])),
        Some("w1:p2".into())
    );
    assert_eq!(focused_pane(&env(&[("HERDR_PANE_ID", "")])), None);
}
