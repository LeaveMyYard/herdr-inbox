use std::path::PathBuf;

use super::*;

#[test]
fn the_checkout_is_the_nearest_folder_with_the_inbox_manifest() {
    let d = tempfile::tempdir().unwrap();
    let bin = d.path().join("target/release");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("herdr-inbox");
    std::fs::write(&exe, "").unwrap();
    assert_eq!(own_checkout(&exe), None);
    let other = "id = \"leavemyyard.herdr-panorama\"\n";
    std::fs::write(d.path().join("herdr-plugin.toml"), other).unwrap();
    assert_eq!(own_checkout(&exe), None, "another plugin's tree");
    let inbox = d.path().join("target");
    let manifest = "# c\nid = \"leavemyyard.herdr-inbox\"\n[x]\n";
    std::fs::write(inbox.join("herdr-plugin.toml"), manifest).unwrap();
    let root = std::fs::canonicalize(&inbox).unwrap();
    assert_eq!(
        own_checkout(&exe),
        Some(root.clone()),
        "the nearest inbox manifest"
    );
    let link = d.path().join("link");
    std::os::unix::fs::symlink(&exe, &link).unwrap();
    assert_eq!(own_checkout(&link), Some(root), "through a symlink");
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
