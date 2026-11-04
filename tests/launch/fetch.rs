//! The MCP launch: it fetches the release for the hooks in the background, then starts the
//! server.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::release::{publish, release_toml};
use crate::world::{Root, stdout};

/// A stand-in `uv` (the MCP server) first on PATH; returns that PATH.
fn with_uv(r: &Root) -> String {
    let dir = r.dir.path().join("fakeuv");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("uv"), "#!/bin/sh\necho \"uv $*\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir.join("uv"), fs::Permissions::from_mode(0o755)).unwrap();
    format!(
        "{}:{}:{}",
        dir.display(),
        r.dir.path().join("tools").display(),
        std::env::var("PATH").unwrap()
    )
}

fn wait_for(path: &Path, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    path.exists()
}

#[test]
fn the_mcp_server_starts_at_once_while_the_release_downloads() {
    let r = Root::new();
    let sha = publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &sha);
    let path = with_uv(&r);
    let started = Instant::now();
    let out = r.sh(
        "launch",
        &["mcp"],
        "",
        &[
            ("PATH", Path::new(&path)),
            ("FAKE_CURL_SLEEP", Path::new("3")),
        ],
    );
    let took = started.elapsed();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout(&out).starts_with("uv run --quiet --frozen --no-dev --project "),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).trim_end().ends_with(" inbox mcp"));
    assert!(
        took < Duration::from_millis(2500),
        "the download (3 s) must not hold the server: {took:?}"
    );
    let fetched = r.data_bin().join("herdr-inbox-0.11.0");
    assert!(
        wait_for(&fetched, Duration::from_secs(20)),
        "fetched in the background"
    );
    assert_eq!(
        stdout(&r.sh("launch", &["hook", "stop"], "", &[])),
        "fetched stop\n",
        "the hooks find it from now on"
    );
}

#[test]
fn no_release_toml_no_fetch() {
    let r = Root::new();
    let path = with_uv(&r);
    let out = r.sh("launch", &["mcp"], "", &[("PATH", Path::new(&path))]);
    assert_eq!(out.status.code(), Some(0));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!r.tool_log().contains("curl"));
}

#[test]
fn concurrent_mcp_starts_download_once() {
    let r = Root::new();
    let sha = publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &sha);
    let path = with_uv(&r);
    let env = [
        ("PATH", Path::new(&path)),
        ("FAKE_CURL_SLEEP", Path::new("2")),
    ];
    for _ in 0..3 {
        assert_eq!(r.sh("launch", &["mcp"], "", &env).status.code(), Some(0));
    }
    let fetched = r.data_bin().join("herdr-inbox-0.11.0");
    assert!(wait_for(&fetched, Duration::from_secs(20)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(r.tool_log().matches("curl ").count(), 1, "{}", r.tool_log());
    assert!(
        !r.data_bin().join(".fetch-lock").exists(),
        "the lock goes with the download"
    );
}

#[test]
fn a_fetch_prunes_the_fetched_copies() {
    // #72: the MCP launch keeps the newest three of each contract after its fetch (another
    // contract here, or the launch would pick one and fetch nothing)
    let r = Root::new();
    for v in ["0.9.0", "0.10.0", "0.10.1", "0.10.2"] {
        crate::world::stand_in(&r.data_bin().join(format!("herdr-inbox-{v}")), v, v, 9, 0);
    }
    let sha = publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &sha);
    let path = with_uv(&r);
    let out = r.sh("launch", &["mcp"], "", &[("PATH", Path::new(&path))]);
    assert_eq!(out.status.code(), Some(0));
    let fetched = r.data_bin().join("herdr-inbox-0.11.0");
    assert!(wait_for(&fetched, Duration::from_secs(20)));
    // the prune runs after the fetch, before the lock goes
    let lock = r.data_bin().join(".fetch-lock");
    let deadline = Instant::now() + Duration::from_secs(20);
    while lock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!lock.exists() && fetched.is_file());
    assert!(
        !r.data_bin().join("herdr-inbox-0.9.0").exists(),
        "the fourth of contract 9 goes"
    );
    assert!(r.data_bin().join("herdr-inbox-0.10.0").exists());
}
