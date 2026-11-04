use serde_json::json;

use super::*;

fn settings(v: serde_json::Value) -> Settings {
    from_table(v.as_object().unwrap())
}

#[test]
fn the_prefix_is_the_default_mark_unless_set_and_is_stripped() {
    assert_eq!(settings(json!({})).prefix, "❓");
    assert_eq!(settings(json!({"tab_prefix": ""})).prefix, "");
    assert_eq!(settings(json!({"tab_prefix": "  ! \u{1f}"})).prefix, "!");
    // Python's str(): tab_prefix = 7 / true
    assert_eq!(settings(json!({"tab_prefix": 7})).prefix, "7");
    assert_eq!(settings(json!({"tab_prefix": true})).prefix, "True");
    assert_eq!(settings(json!({"tab_prefix": 2.0})).prefix, "2.0");
}

#[test]
fn prefix_unnamed_tabs_is_pythons_truthiness() {
    assert!(!settings(json!({})).prefix_unnamed);
    for on in [json!(true), json!(1), json!("no"), json!([0])] {
        assert!(
            settings(json!({"prefix_unnamed_tabs": on})).prefix_unnamed,
            "{on}"
        );
    }
    for off in [json!(false), json!(0), json!(""), json!([]), json!({})] {
        assert!(
            !settings(json!({"prefix_unnamed_tabs": off})).prefix_unnamed,
            "{off}"
        );
    }
}

#[test]
fn a_broken_file_is_no_settings_and_is_warned_about() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "tab_prefix = [\n").unwrap();
    let mut config = ConfigFile::in_dir(dir.path());
    let mut warned = Vec::new();
    let got = read(&mut config, &mut |m| warned.push(m.to_string()));
    assert_eq!(got.prefix, "❓");
    assert_eq!(warned.len(), 1);
    assert!(warned[0].starts_with("ignoring "), "{warned:?}");
}
