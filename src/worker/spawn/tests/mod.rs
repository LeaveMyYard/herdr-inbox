//! Python tests/test_successor.py and the spawn half of
//! tests/test_stale_worker.py, against stand-in processes ([`fakes`]).

mod ensure;
mod fakes;
mod successor;

use std::fs;
use std::os::unix::fs::symlink;

use super::*;
use fakes::{Fakes, write_exe};

#[test]
fn target_release_is_preferred_over_bin() {
    let f = Fakes::new();
    let root = f.root("checkout");
    assert_eq!(rust_worker_binary(&root), None);
    let bin = write_exe(&root.join("bin/herdr-inbox"), "#!/bin/sh\n");
    assert_eq!(rust_worker_binary(&root), Some(bin));
    let built = write_exe(&root.join("target/release/herdr-inbox"), "#!/bin/sh\n");
    assert_eq!(rust_worker_binary(&root), Some(built));
}

#[test]
fn only_an_executable_file_counts_as_installed() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fakes::new();
    let not_exec = f.root("not-executable");
    let path = write_exe(&not_exec.join("target/release/herdr-inbox"), "#!/bin/sh\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let dir = f.root("directory");
    fs::create_dir_all(dir.join("target/release/herdr-inbox")).unwrap();
    let dangling = f.root("dangling");
    fs::create_dir_all(dangling.join("target/release")).unwrap();
    symlink(
        dangling.join("nowhere"),
        dangling.join("target/release/herdr-inbox"),
    )
    .unwrap();
    for root in [not_exec, dir, dangling] {
        assert_eq!(rust_worker_binary(&root), None, "{}", root.display());
    }
}

#[test]
fn a_symlinked_binary_counts() {
    let f = Fakes::new();
    let root = f.root("checkout");
    let real = write_exe(&root.join("elsewhere/herdr-inbox"), "#!/bin/sh\n");
    let link = root.join("target/release/herdr-inbox");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    symlink(real, &link).unwrap();
    assert_eq!(rust_worker_binary(&root), Some(link));
}

#[test]
fn the_successor_commands_are_pythons() {
    let root = std::path::Path::new("/install/r2");
    let uv: Vec<_> = uv_worker_args(root)
        .into_iter()
        .map(|a| a.into_string().unwrap())
        .collect();
    assert_eq!(
        uv,
        [
            "run",
            "--quiet",
            "--frozen",
            "--no-dev",
            "--project",
            "/install/r2",
            "inbox",
            "worker"
        ]
    );
    let py: Vec<_> = python_worker_args(&root.join("src"))
        .into_iter()
        .map(|a| a.into_string().unwrap())
        .collect();
    assert_eq!(py[..2], ["-I", "-c"]);
    assert_eq!(
        py[2],
        "import runpy, sys; sys.path.insert(0, sys.argv.pop(1)); \
         runpy.run_module('herdr_inbox.cli', run_name='__main__', alter_sys=True)"
    );
    assert_eq!(py[3..], ["/install/r2/src", "worker"]);
    assert_eq!(rust_worker_args(), ["worker"]);
}

/// Python test_ensure_starts_at_most_one_worker_per_revision_per_interval
/// (the stamp itself).
#[test]
fn the_spawn_stamp_limits_each_revision_and_ignores_the_future() {
    let f = Fakes::new();
    let lock = f.dir.path().join("state").join("worker-x.lock");
    assert!(may_spawn(&lock, 1), "its directory created");
    let stamp = stamp_path(&lock, 1);
    assert_eq!(stamp, f.dir.path().join("state/worker-x.lock.spawned-1"));
    assert!(
        !may_spawn(&lock, 1) && !may_spawn(&lock, 1),
        "the first may still be starting"
    );
    assert!(may_spawn(&lock, 2), "per revision");
    let file = fs::File::options().append(true).open(&stamp).unwrap();
    let old = std::time::SystemTime::now() - ENSURE_EVERY - std::time::Duration::from_secs(1);
    file.set_modified(old).unwrap();
    assert!(may_spawn(&lock, 1));
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    file.set_modified(future).unwrap();
    assert!(
        may_spawn(&lock, 1),
        "a stamp from the future does not count"
    );
}
