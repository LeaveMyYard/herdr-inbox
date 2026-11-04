use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use super::*;

pub(crate) fn env_at(db: PathBuf, server: &str) -> WorkerEnv {
    WorkerEnv {
        db,
        server: server.into(),
        revision: 1,
        version: "0.10.6".into(),
        waits: Waits::default(),
        spawn_env: Vec::new(),
        no_worker: false,
    }
}

#[test]
fn the_waits_are_pythons_and_read_its_variables() {
    let d = Waits::default();
    assert_eq!(d.tick, Duration::from_secs(5));
    assert_eq!(d.idle_exit, Duration::from_secs(600));
    assert_eq!(d.standby_max, Duration::from_secs(1800));
    assert_eq!(d.handover_wait, Duration::from_secs(5));
    assert_eq!(d.server_gone_exit, Duration::from_secs(10));
    let w = Waits::from_vars(|k| match k {
        "HERDR_INBOX_WORKER_TICK" => Some("0.2".into()),
        "HERDR_INBOX_WORKER_IDLE_EXIT" => Some("120".into()),
        "HERDR_INBOX_STANDBY_MAX" => Some("30".into()),
        _ => None,
    });
    assert_eq!(w.tick, Duration::from_millis(200));
    assert_eq!(w.idle_exit, Duration::from_secs(120));
    assert_eq!(w.standby_max, Duration::from_secs(30));
    assert_eq!(w.handover_wait, d.handover_wait, "not a variable in Python");
    for bad in ["", "x", "-1", "inf", "nan"] {
        let w = Waits::from_vars(|_| Some(bad.into()));
        assert_eq!(w, d, "{bad:?} keeps the defaults");
    }
}

/// Python test_delivery_worker::test_m1_the_lock_is_per_herdr_server.
#[test]
fn the_lock_is_per_herdr_server_beside_the_database() {
    let a = env_at("/state/inbox.db".into(), "/run/a.sock");
    let b = env_at("/state/inbox.db".into(), "/run/b.sock");
    assert_ne!(a.lock_path(), b.lock_path());
    // sha1("/run/a.sock")[:12], as Python's hashlib names it
    let name = a
        .lock_path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        name.starts_with("worker-") && name.ends_with(".lock") && name.len() == 24,
        "{name}"
    );
    assert_eq!(a.lock_path().parent(), Some(std::path::Path::new("/state")));
}

/// Python test_delivery_worker::test_m2_spawns_do_not_inherit_the_mcp_venv.
#[test]
fn spawns_do_not_inherit_the_venv_and_name_the_server() {
    let vars: Vec<(OsString, OsString)> = [
        ("PATH", "/bin"),
        ("UV_PROJECT_ENVIRONMENT", "/data/venv-0.10.5"),
        ("UV_CACHE_DIR", "/c"),
        ("VIRTUAL_ENV", "/v"),
        ("HERDR_SOCKET_PATH", "/other.sock"),
        ("XUV_KEEP", "1"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let got = spawn_env(vars.clone(), "/run/a.sock".as_ref());
    let keys: Vec<_> = got
        .iter()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    assert_eq!(keys, ["PATH", "XUV_KEEP", "HERDR_SOCKET_PATH"]);
    assert_eq!(got[2].1, "/run/a.sock");
    let inherited = spawn_env(vars, "".as_ref());
    let socket: Vec<_> = inherited
        .iter()
        .filter(|(k, _)| k == "HERDR_SOCKET_PATH")
        .collect();
    assert_eq!(socket.len(), 1, "no server given: the inherited one stays");
    assert_eq!(socket[0].1, "/other.sock");
}
