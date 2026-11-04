use herdr_agent_kit::herdr_api::fake::{CliReply, CliStub, FakeHerdr};

use super::*;

/// review #118 M1: the plugin's settings are looked up on the first marker refresh,
/// never when the server starts (a slow herdr would hold up the MCP handshake).
#[test]
fn the_settings_are_found_on_the_first_refresh_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    Store::open_migrating(&db, Duration::from_secs(3)).unwrap();
    let fake = FakeHerdr::start();
    let mut cli = CliStub::new();
    cli.on(
        &["plugin", "config-dir"],
        CliReply::ok(&dir.path().join("cfg").to_string_lossy()),
    );
    let herdr = fake.blocking_client().with_bin(cli.bin());
    let env = PluginEnv::from_vars(
        Vec::<(String, String)>::new(),
        Some(dir.path().to_path_buf()),
        1000,
    );
    let herald = Live::new(herdr, env, db);
    let config_lookups = || {
        cli.calls()
            .iter()
            .filter(|c| c.first().is_some_and(|a| a == "plugin"))
            .count()
    };
    assert_eq!(config_lookups(), 0, "nothing at start");
    herald.markers("w1:p1");
    herald.markers("w1:p1");
    assert_eq!(config_lookups(), 1, "once, then kept");
}
