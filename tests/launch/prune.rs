//! #72: the recorded binaries do not pile up, one per release, forever.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::release::{publish, release_toml};
use crate::world::{Root, repo, stand_in};

fn names(dir: &Path) -> Vec<String> {
    let mut n: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    n.sort();
    n
}

/// `inbox_prune <dir> <keep> [<path>]` from scripts/release-lib.sh.
fn prune(dir: &Path, keep: u32, protect: &str) {
    let lib = repo().join("scripts/release-lib.sh");
    let script = format!(". '{}'; inbox_prune \"$1\" {keep} \"$2\"", lib.display());
    let ok = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(dir)
        .arg(protect)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .status()
        .unwrap();
    assert!(ok.success());
}

#[test]
fn the_newest_of_each_contract_stay_and_nothing_else_is_touched() {
    let r = Root::new();
    let bin = r.state_bin();
    for v in ["0.9.0", "0.9.10", "0.10.0", "0.10.2", "0.9.9"] {
        stand_in(&bin.join(format!("herdr-inbox-{v}")), v, v, 2, 0);
    }
    for v in ["0.12.0", "0.12.1"] {
        stand_in(&bin.join(format!("herdr-inbox-{v}")), v, v, 1, 0);
    }
    fs::write(bin.join("notes.txt"), "kept").unwrap();
    fs::write(bin.join(".herdr-inbox-0.1.0.123"), "a copy in flight").unwrap();
    prune(&bin, 3, "");
    assert_eq!(
        names(&bin),
        [
            ".herdr-inbox-0.1.0.123",
            "herdr-inbox-0.10.0",
            "herdr-inbox-0.10.2",
            "herdr-inbox-0.12.0",
            "herdr-inbox-0.12.1",
            "herdr-inbox-0.9.10",
            "notes.txt",
        ],
        "numeric order; an older contract keeps its own newest"
    );
}

#[test]
fn the_binary_just_recorded_is_never_removed() {
    let r = Root::new();
    let bin = r.state_bin();
    for v in ["1.0.0", "1.0.1", "0.0.1"] {
        stand_in(&bin.join(format!("herdr-inbox-{v}")), v, v, 2, 0);
    }
    let old = bin.join("herdr-inbox-0.0.1");
    prune(&bin, 1, old.to_str().unwrap());
    assert_eq!(names(&bin), ["herdr-inbox-0.0.1", "herdr-inbox-1.0.1"]);
}

#[test]
fn an_install_prunes_the_recorded_copies() {
    let r = Root::new();
    for v in ["0.10.0", "0.10.1", "0.10.2", "0.10.3"] {
        stand_in(&r.state_bin().join(format!("herdr-inbox-{v}")), v, v, 2, 0);
    }
    let sha = publish(&r, "0.11.0");
    release_toml(&r, "0.11.0", &sha);
    let out = r.sh("install-bin.sh", &[], "", &[]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        names(&r.state_bin()),
        [
            "herdr-inbox-0.10.2",
            "herdr-inbox-0.10.3",
            "herdr-inbox-0.11.0"
        ]
    );
}
