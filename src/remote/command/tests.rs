use super::*;

#[test]
fn targets_are_pythons_valid_target() {
    // Python: valid_target(t) for each
    for (t, ok) in [
        ("box.lan", true),
        ("user@host:22", true),
        ("[::1]", true),
        ("a-b_c.d@e", true),
        (&"x".repeat(253), true),
        ("-oProxyCommand=x", false),
        ("a b", false),
        ("a\tb", false),
        ("", false),
        ("é.lan", false),
        ("host;rm", false),
        ("user@host=1", false),
        (&"x".repeat(254), false),
        ("a/b", false),
    ] {
        assert_eq!(valid_target(t), ok, "{t:?}");
        assert_eq!(SafeTarget::for_ssh(Some(t)).is_ok(), ok, "{t:?}");
    }
    assert_eq!(
        SafeTarget::for_ssh(Some("a b")),
        Err("refusing ssh target 'a b'".into())
    );
    assert_eq!(
        SafeTarget::for_ssh(None),
        Err("refusing ssh target None".into())
    );
    assert_eq!(
        SafeTarget::for_call(Some("-p22")),
        Err("refusing that ssh target".into())
    );
}

#[test]
fn a_plugin_list_needs_a_safe_herdr() {
    assert!(FixedCommand::plugin_list("/usr/bin/herdr").is_some());
    assert!(FixedCommand::plugin_list("herdr; rm -rf /").is_none());
}

#[test]
fn the_inbox_command_is_pythons_byte_for_byte() {
    // Python: remote.remote_command(rec, "export" | "apply") for these paths; any other sub asserts
    let rec = Machine {
        uv: Some("/home/u/.local/bin/uv".into()),
        herdr: Some("/usr/local/bin/herdr".into()),
        plugin_root: Some("/home/u/.config/herdr/plugins/inbox".into()),
        ..Machine::default()
    };
    assert_eq!(
        FixedCommand::inbox(&rec, InboxSub::Export)
            .unwrap()
            .as_str(),
        "env HERDR_BIN_PATH=/usr/local/bin/herdr /home/u/.local/bin/uv run --quiet --frozen --no-dev \
         --project /home/u/.config/herdr/plugins/inbox inbox export"
    );
    assert_eq!(
        FixedCommand::inbox(&rec, InboxSub::Apply).unwrap().as_str(),
        "env HERDR_BIN_PATH=/usr/local/bin/herdr /home/u/.local/bin/uv run --quiet --frozen --no-dev \
         --project /home/u/.config/herdr/plugins/inbox inbox apply"
    );
    let bad = Machine {
        uv: Some("uv".into()),
        ..rec.clone()
    };
    assert_eq!(
        FixedCommand::inbox(&bad, InboxSub::Export),
        Err("bad uv".into())
    );
}
