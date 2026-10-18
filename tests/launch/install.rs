//! scripts/install-bin.sh and the MCP launch's fetch, against a `file://` release.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::world::{Root, stand_in, stdout};

/// This machine's release target, as scripts/release-lib.sh names it.
fn target() -> String {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            ". {}; inbox_target",
            crate::world::repo()
                .join("scripts/release-lib.sh")
                .display()
        ))
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn sha256(path: &Path) -> String {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "(sha256sum || shasum -a 256) < '{}'",
            path.display()
        ))
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

/// A published release v<version> under `dir` (file://), with a stand-in binary; returns
/// the release base URL and the tarball's sha256.
fn publish(dir: &Path, version: &str) -> (String, String) {
    let stage = dir.join("stage");
    stand_in(&stage.join("herdr-inbox"), "fetched", version, 1, 0);
    let rel = dir.join(format!("v{version}"));
    fs::create_dir_all(&rel).unwrap();
    let tarball = rel.join(format!("herdr-inbox-v{version}-{}.tar.gz", target()));
    let ok = Command::new("tar")
        .arg("-C")
        .arg(&stage)
        .arg("-czf")
        .arg(&tarball)
        .arg("herdr-inbox")
        .status()
        .unwrap();
    assert!(ok.success());
    (format!("file://{}", dir.display()), sha256(&tarball))
}

fn release_toml(root: &Path, version: &str, sha: &str) {
    fs::write(
        root.join("release.toml"),
        format!(
            "tag = \"v{version}\"\n\n[sha256]\n{} = \"{sha}\"\n",
            target()
        ),
    )
    .unwrap();
}

fn no_cargo_path(dir: &Path) -> PathBuf {
    // a PATH with the tools the scripts use but no cargo
    let bin = dir.join("nocargo");
    fs::create_dir_all(&bin).unwrap();
    for tool in [
        "sh",
        "sed",
        "head",
        "cut",
        "uname",
        "mktemp",
        "mkdir",
        "dirname",
        "curl",
        "tar",
        "gzip",
        "chmod",
        "mv",
        "rm",
        "cp",
        "sha256sum",
        "shasum",
        "cat",
    ] {
        if let Ok(out) = Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {tool}"))
            .output()
        {
            let p = String::from_utf8(out.stdout).unwrap();
            if !p.trim().is_empty() {
                let _ = std::os::unix::fs::symlink(p.trim(), bin.join(tool));
            }
        }
    }
    bin
}

#[test]
fn install_fetches_the_release_checks_it_and_records_a_copy_for_claude() {
    let r = Root::new();
    let (base, sha) = publish(&r.dir.path().join("releases"), "0.11.0");
    release_toml(&r.root(), "0.11.0", &sha);
    let base = PathBuf::from(base);
    let out = r.sh(
        "install-bin.sh",
        &[],
        "",
        &[("HERDR_INBOX_RELEASE_BASE", &base)],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bin = r.root().join("bin/herdr-inbox");
    assert_eq!(
        stdout(&Command::new(&bin).output().unwrap()),
        "herdr-inbox 0.11.0\n"
    );
    assert!(
        r.state_bin().join("herdr-inbox-0.11.0").is_file(),
        "the hard copy outside the install dir"
    );
    let leftovers: Vec<_> = fs::read_dir(r.root().join("bin"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["herdr-inbox"], "no temporary files");
    // and the launcher finds the recorded copy
    assert_eq!(
        stdout(&r.sh("launch", &["hook", "stop"], "", &[])),
        "fetched stop\n"
    );
}

#[test]
fn a_checksum_mismatch_fails_the_install_and_installs_nothing() {
    let r = Root::new();
    let (base, _) = publish(&r.dir.path().join("releases"), "0.11.0");
    release_toml(&r.root(), "0.11.0", &"0".repeat(64));
    let out = r.sh(
        "install-bin.sh",
        &[],
        "",
        &[("HERDR_INBOX_RELEASE_BASE", Path::new(&base))],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("sha256"));
    assert!(
        !r.root().join("bin/herdr-inbox").exists()
            && fs::read_dir(r.state_bin()).unwrap().next().is_none()
    );
}

#[test]
fn no_asset_and_no_cargo_is_a_clear_error() {
    let r = Root::new();
    let path = no_cargo_path(r.dir.path());
    // release.toml names a release with no asset for this machine
    let toml = "tag = \"v0.11.0\"\n\n[sha256]\nnone-such-target = \"00\"\n";
    fs::write(r.root().join("release.toml"), toml).unwrap();
    let out = r.sh("install-bin.sh", &[], "", &[("PATH", &path)]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no cargo"), "{err}");
}

#[test]
fn before_the_first_release_without_cargo_the_install_still_succeeds() {
    let r = Root::new();
    let path = no_cargo_path(r.dir.path());
    let out = r.sh("install-bin.sh", &[], "", &[("PATH", &path)]);
    assert_eq!(out.status.code(), Some(0), "the Python hooks stay in use");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Python hooks"));
    assert!(!r.root().join("bin/herdr-inbox").exists());
}

#[test]
fn the_mcp_launch_fetches_the_release_for_the_hooks_and_starts_the_server() {
    let r = Root::new();
    let (base, sha) = publish(&r.dir.path().join("releases"), "0.11.0");
    release_toml(&r.root(), "0.11.0", &sha);
    // a stand-in uv: the MCP server
    let fake = r.dir.path().join("fakeuv");
    fs::create_dir_all(&fake).unwrap();
    stand_in(&fake.join("uv"), "uv", "0", 0, 0);
    fs::write(fake.join("uv"), "#!/bin/sh\necho \"uv $*\"\n").unwrap();
    let path = format!("{}:{}", fake.display(), std::env::var("PATH").unwrap());
    let out = r.sh(
        "launch",
        &["mcp"],
        "",
        &[
            ("PATH", Path::new(&path)),
            ("HERDR_INBOX_RELEASE_BASE", Path::new(&base)),
        ],
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout(&out).starts_with("uv run --quiet --frozen --no-dev --project "),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).trim_end().ends_with(" inbox mcp"));
    let fetched = r.data_bin().join("herdr-inbox-0.11.0");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !fetched.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(fetched.is_file(), "fetched in the background");
    assert_eq!(
        stdout(&r.sh("launch", &["hook", "stop"], "", &[])),
        "fetched stop\n",
        "the hooks find it from now on"
    );
}
