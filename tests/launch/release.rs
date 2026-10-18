//! A published release for the stand-in curl: `<releases>/v<version>/<tarball>`.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::world::{Root, repo, stand_in};

/// This machine's release target, as scripts/release-lib.sh names it.
pub fn target() -> String {
    let lib = repo().join("scripts/release-lib.sh");
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(". '{}'; inbox_target", lib.display()))
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn sha256(path: &Path) -> String {
    let script = format!("(sha256sum || shasum -a 256) < '{}'", path.display());
    let out = Command::new("sh").arg("-c").arg(script).output().unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

/// Publishes v<version> (a stand-in binary named "fetched") and returns its tarball's sha256.
pub fn publish(r: &Root, version: &str) -> String {
    let stage = r.dir.path().join("stage");
    stand_in(&stage.join("herdr-inbox"), "fetched", version, 1, 0);
    let rel = r.releases().join(format!("v{version}"));
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
    sha256(&tarball)
}

/// release.toml naming v<version>, with `sha` for this machine's target.
pub fn release_toml(r: &Root, version: &str, sha: &str) {
    let text = format!(
        "tag = \"v{version}\"\n\n[sha256]\n{} = \"{sha}\"\n",
        target()
    );
    fs::write(r.root().join("release.toml"), text).unwrap();
}
