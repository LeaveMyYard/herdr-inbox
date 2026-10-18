//! scripts/install-bin.sh against the stand-in curl and cargo (tests/launch/world.rs).

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::release::{publish, release_toml};
use crate::world::{Root, stdout};

fn install(r: &Root, extra: &[(&str, &Path)]) -> (Option<i32>, String) {
    let out = r.sh("install-bin.sh", &[], "", extra);
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn leftovers(dir: &Path) -> Vec<String> {
    fs::read_dir(dir).map_or(Vec::new(), |d| {
        d.map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect()
    })
}

#[test]
fn install_fetches_the_release_checks_it_and_records_a_copy_for_claude() {
    let r = Root::new();
    let sha = publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &sha);
    let (code, err) = install(&r, &[]);
    assert_eq!(code, Some(0), "{err}");
    let bin = r.root().join("bin/herdr-inbox");
    assert_eq!(
        stdout(&Command::new(&bin).output().unwrap()),
        "herdr-inbox 0.11.0\n"
    );
    assert!(
        r.state_bin().join("herdr-inbox-0.11.0").is_file(),
        "the hard copy outside the install dir"
    );
    assert_eq!(
        leftovers(&r.root().join("bin")),
        ["herdr-inbox"],
        "no temporary files"
    );
    let log = r.tool_log();
    assert!(
        log.contains("--proto =https --proto-redir =https"),
        "HTTPS only: {log}"
    );
    assert!(log.contains("--max-time"), "{log}");
    assert_eq!(
        stdout(&r.sh("launch", &["hook", "stop"], "", &[])),
        "fetched stop\n",
        "the launcher finds it"
    );
}

#[test]
fn a_checksum_mismatch_fails_the_install_and_leaves_nothing() {
    let r = Root::new();
    publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &"0".repeat(64));
    let (code, err) = install(&r, &[]);
    assert_eq!(code, Some(1));
    assert!(err.contains("sha256"), "{err}");
    assert!(
        leftovers(&r.root().join("bin")).is_empty(),
        "no binary, no .fetch.* directory"
    );
    assert!(leftovers(&r.state_bin()).is_empty());
    assert!(
        !r.tool_log().contains("cargo"),
        "never built around a mismatch"
    );
}

#[test]
fn no_sha256_tool_installs_nothing_and_says_so() {
    let r = Root::new();
    let sha = publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &sha);
    // a PATH with the stand-ins and the basic tools, but no sha256sum or shasum
    let bin = r.dir.path().join("nosha");
    fs::create_dir_all(&bin).unwrap();
    for tool in [
        "sh", "sed", "head", "cut", "uname", "mktemp", "mkdir", "dirname", "tar", "gzip", "chmod",
        "mv", "rm", "cp", "cat", "sleep",
    ] {
        let found = Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {tool}"))
            .output()
            .unwrap();
        let found = String::from_utf8(found.stdout).unwrap();
        if !found.trim().is_empty() {
            std::os::unix::fs::symlink(found.trim(), bin.join(tool)).unwrap();
        }
    }
    let path = format!("{}:{}", r.dir.path().join("tools").display(), bin.display());
    let (code, err) = install(&r, &[("PATH", Path::new(&path))]);
    assert_eq!(code, Some(1));
    assert!(err.contains("no sha256sum or shasum"), "{err}");
    assert!(leftovers(&r.root().join("bin")).is_empty());
}

#[test]
fn before_the_first_release_nothing_is_built_or_installed() {
    let r = Root::new();
    let target = r.dir.path().join("target");
    let (code, err) = install(
        &r,
        &[
            ("FAKE_CARGO_OK", Path::new("1")),
            ("FAKE_TARGET_DIR", &target),
        ],
    );
    assert_eq!(code, Some(0));
    assert!(err.contains("Python hooks"), "{err}");
    assert_eq!(r.tool_log(), "", "no download, no build");
    assert!(!r.root().join("bin").exists());
}

#[test]
fn a_failed_download_installs_nothing_and_builds_nothing() {
    let r = Root::new();
    release_toml(&r, "0.11.0", &"0".repeat(64)); // nothing published: the download fails
    let target = r.dir.path().join("target");
    let (code, err) = install(
        &r,
        &[
            ("FAKE_CARGO_OK", Path::new("1")),
            ("FAKE_TARGET_DIR", &target),
        ],
    );
    assert_eq!(code, Some(0), "never fails the herdr install");
    assert!(err.contains("download failed"), "{err}");
    assert!(!r.tool_log().contains("cargo"));
    assert!(leftovers(&r.root().join("bin")).is_empty());
}

#[test]
fn no_asset_for_this_machine_builds_where_cargo_builds_or_stays_python() {
    let r = Root::new();
    fs::write(
        r.root().join("release.toml"),
        "tag = \"v0.11.0\"\n\n[sha256]\nnone-such-target = \"00\"\n",
    )
    .unwrap();
    // no cargo build possible: exit 0, Python hooks
    let (code, err) = install(&r, &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("no release binary for this machine") && err.contains("build failed"),
        "{err}"
    );
    // a build into cargo's own target directory (a global build.target-dir)
    let target = r.dir.path().join("elsewhere");
    let (code, err) = install(
        &r,
        &[
            ("FAKE_CARGO_OK", Path::new("1")),
            ("FAKE_TARGET_DIR", &target),
        ],
    );
    assert_eq!(code, Some(0), "{err}");
    assert!(
        r.tool_log().contains("cargo metadata")
            && r.tool_log().contains("cargo build --release --locked")
    );
    assert!(r.state_bin().join("herdr-inbox-0.0.7").is_file());
}

#[test]
fn an_update_replaces_the_binary_by_rename_never_in_place() {
    // a running binary keeps its inode: another name for the old file still shows the old one
    let r = Root::new();
    let sha = publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &sha);
    let bin = r.root().join("bin/herdr-inbox");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    fs::write(&bin, "old binary").unwrap();
    let running = r.dir.path().join("running");
    fs::hard_link(&bin, &running).unwrap();
    assert_eq!(install(&r, &[]).0, Some(0));
    assert_eq!(fs::read_to_string(&running).unwrap(), "old binary");
    assert_eq!(
        stdout(&Command::new(&bin).output().unwrap()),
        "herdr-inbox 0.11.0\n"
    );
}

#[test]
fn no_home_and_no_state_dir_still_exits_0() {
    let r = Root::new();
    let sha = publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &sha);
    let path = format!(
        "{}:{}",
        r.dir.path().join("tools").display(),
        std::env::var("PATH").unwrap()
    );
    let out = Command::new("sh")
        .arg(r.root().join("scripts/install-bin.sh"))
        .env_clear()
        .env("PATH", path)
        .env("FAKE_RELEASES", r.releases())
        .env("FAKE_LOG", r.dir.path().join("tools.log"))
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(r.root().join("bin/herdr-inbox").is_file());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not recorded"));
}
