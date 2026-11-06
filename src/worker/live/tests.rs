//! The real world's answers (Python tests/test_successor.py, "herdr's
//! current plugin_root", and tests/test_stale_worker.py's version check).

use std::fs;
use std::path::{Path, PathBuf};

use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use serde_json::json;

use super::*;
use crate::store::revision::REVISION_BASE;

fn install(dir: &Path, name: &str, version: &str) -> PathBuf {
    let root = dir.join("installs").join(name);
    fs::create_dir_all(root.join("src/herdr_inbox")).unwrap();
    fs::write(
        root.join("herdr-plugin.toml"),
        format!("version = \"{version}\"\n"),
    )
    .unwrap();
    let pyproject = format!("[project]\nname = \"herdr-inbox\"\nversion = \"{version}\"\n");
    fs::write(root.join("pyproject.toml"), pyproject).unwrap();
    root
}

fn lists(fake: &FakeHerdr, root: Option<&Path>) {
    let plugins = match root {
        Some(r) => json!([
            {"plugin_id": "someone.else", "plugin_root": "/elsewhere"},
            {"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": r},
        ]),
        None => json!([]),
    };
    fake.on("plugin.list", Reply::Result(json!({"plugins": plugins})));
}

/// Python test_a_worker_without_package_metadata_knows_its_version: the
/// version the worker compares is its checkout's, the binary's own.
#[test]
fn a_worker_knows_its_version_from_its_checkout() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(version_at(here).as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(version_at(Path::new("/nonexistent")), None);
    let exe = here.join("target/release/herdr-inbox");
    assert_eq!(own_checkout(&exe).as_deref(), Some(here));
    assert_eq!(
        own_checkout(&here.join("bin/herdr-inbox")).as_deref(),
        Some(here)
    );
    assert_eq!(own_checkout(Path::new("/usr/local/bin/herdr-inbox")), None);
}

#[test]
fn herdrs_own_install_follows_the_listed_root_uncached() {
    let dir = tempfile::tempdir().unwrap();
    let own = install(dir.path(), "own", "0.10.6");
    let fake = FakeHerdr::start();
    let herdr = fake.blocking_client().with_bin("/nonexistent/herdr-cli");
    let mut it = Install::new(Some(own.clone()));
    lists(&fake, None);
    assert_eq!(
        it.current(|| listed_root(&herdr)),
        Some(own.clone()),
        "herdr lists nothing"
    );
    assert!(
        !it.follows(|| listed_root(&herdr)),
        "no answer yet: undecided, no"
    );
    lists(&fake, Some(&own));
    assert_eq!(it.current(|| listed_root(&herdr)), Some(own.clone()));
    let (first, second) = (
        install(dir.path(), "a", "1.0.0"),
        install(dir.path(), "b", "1.1.0"),
    );
    lists(&fake, Some(&first));
    assert_eq!(
        it.current(|| listed_root(&herdr)),
        Some(first),
        "herdr wins over its own path"
    );
    lists(&fake, Some(&second));
    assert_eq!(
        it.current(|| listed_root(&herdr)),
        Some(second.clone()),
        "not cached"
    );
    assert_eq!(version_at(&second).as_deref(), Some("1.1.0"));
}

#[test]
fn another_checkout_never_follows_the_listed_root() {
    let dir = tempfile::tempdir().unwrap();
    let own = install(dir.path(), "worktree", "0.10.6");
    let other = install(dir.path(), "main", "0.1.0");
    let fake = FakeHerdr::start();
    let herdr = fake.blocking_client().with_bin("/nonexistent/herdr-cli");
    lists(&fake, Some(&other));
    let mut it = Install::new(Some(own.clone()));
    assert!(!it.follows(|| listed_root(&herdr)));
    lists(&fake, Some(&own));
    assert!(!it.follows(|| listed_root(&herdr)), "decided once");
    assert_eq!(it.current(|| listed_root(&herdr)), Some(own));
}

#[test]
fn an_installed_copy_without_a_checkout_follows_herdr() {
    let dir = tempfile::tempdir().unwrap();
    let listed = install(dir.path(), "x", "2.0.0");
    let fake = FakeHerdr::start();
    let herdr = fake.blocking_client().with_bin("/nonexistent/herdr-cli");
    lists(&fake, Some(&listed));
    let mut it = Install::new(None);
    assert!(it.follows(|| None));
    assert_eq!(it.current(|| listed_root(&herdr)), Some(listed));
}

/// Python test_server_gone_needs_herdrs_own_word: gone, up, another error,
/// a reply that is no JSON.
#[test]
fn server_gone_needs_herdrs_own_word() {
    let fake = FakeHerdr::start();
    let herdr = fake.blocking_client().with_bin("/nonexistent/herdr-cli");
    fake.on("workspace.list", Reply::Result(json!({"workspaces": []})));
    assert!(!server_gone(&herdr), "up");
    fake.on("workspace.list", Reply::error("timeout", "slow"));
    assert!(!server_gone(&herdr), "another error");
    fake.on(
        "workspace.list",
        Reply::Raw(b"herdr: something odd\n".to_vec()),
    );
    assert!(!server_gone(&herdr), "not json");
    drop(fake);
    assert!(
        server_gone(&herdr),
        "its socket is gone: server_not_running"
    );
}

/// Python test_server_gone_without_a_herdr_binary_is_not_gone, for the
/// socket: one it may not reach (a permission error) is no proof the server
/// is gone.
#[test]
fn an_unreachable_server_is_not_gone() {
    use std::os::unix::fs::PermissionsExt;
    let fake = FakeHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    let socket = locked.join("s");
    std::os::unix::fs::symlink(fake.socket(), &socket).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let herdr = Herdr::new(herdr_agent_kit::herdr_api::ServerId::new(&socket))
        .with_bin("/nonexistent/herdr-cli");
    let gone = server_gone(&herdr);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!gone, "a permission error is not server_not_running");
}

#[test]
fn the_stamp_is_read_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    assert_eq!(db_stamp(&db), None);
    assert!(!db.exists(), "never created");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.pragma_update(None, "user_version", 12345).unwrap();
    assert_eq!(db_stamp(&db), None, "a legacy crc32 stamp");
    conn.pragma_update(None, "user_version", REVISION_BASE + 2)
        .unwrap();
    assert_eq!(db_stamp(&db), Some(2));
}

/// Records the order of what the run loop asks of the duties.
#[derive(Default)]
struct Log(Vec<&'static str>);

impl Duties for Log {
    fn migrate(&mut self) {
        self.0.push("migrate");
    }
    fn tick(&mut self) -> Ticked {
        self.0.push("tick");
        Ticked::default()
    }
    fn pending_now(&mut self) -> usize {
        0
    }
}

/// The remote thread starts in a tick and opens without migrating: the worker's
/// `begin` has migrated by then, before anything else it does.
#[test]
fn the_worker_migrates_when_it_begins_before_any_tick() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeHerdr::start();
    lists(&fake, None);
    let env = WorkerEnv {
        db: dir.path().join("inbox.db"),
        server: "/fake/herdr.sock".into(),
        revision: 1,
        version: "0.0.0".into(),
        waits: crate::worker::Waits::default(),
        spawn_env: Vec::new(),
        no_worker: true,
    };
    let herdr = fake.blocking_client().with_bin("/nonexistent/herdr-cli");
    let mut live = Live::new(
        env,
        herdr,
        Install::new(None),
        "/nonexistent/herdr-inbox".into(),
        Log::default(),
    );
    live.begin();
    live.tick();
    assert_eq!(live.duties().0, ["migrate", "tick"]);
}
