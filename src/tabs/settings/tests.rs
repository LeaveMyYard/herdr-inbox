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

#[test]
fn the_answer_expiry_is_twelve_hours_unless_set() {
    let hours = |v| {
        settings(json!({"remote_answer_expiry_hours": v}))
            .answer_expiry
            .as_secs_f64()
            / 3600.0
    };
    assert_eq!(settings(json!({})).answer_expiry.as_secs(), 12 * 3600);
    // Python: float(value or 12)
    assert_eq!(hours(json!(3)), 3.0);
    assert_eq!(hours(json!(0.5)), 0.5);
    assert_eq!(hours(json!(" 2 ")), 2.0);
    assert_eq!(hours(json!(true)), 1.0);
    for falsy in [json!(0), json!(""), json!(false)] {
        assert_eq!(hours(falsy), 12.0);
    }
    // Python raises or overflows on these: the popup keeps working on the default
    for unusable in [json!(-1), json!("soon"), json!([1]), json!(1e300)] {
        assert_eq!(hours(unusable), 12.0);
    }
}
