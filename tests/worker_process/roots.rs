//! Install roots laid out as herdr lays them out, made from this checkout's
//! Python package (as tests/test_worker_handover.py makes them).

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

const CHECKOUT: &str = env!("CARGO_MANIFEST_DIR");

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "__pycache__" {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&src, &dst);
        } else {
            fs::copy(&src, &dst).unwrap();
        }
    }
}

/// An install root `dir/name` of this checkout's Python worker at `version`
/// (its `pyproject.toml`) and schema `revision` (2 adds one column, as the
/// Python test's r2 does).
pub fn python_root(dir: &Path, name: &str, version: &str, revision: i64) -> PathBuf {
    let root = dir.join(name);
    let checkout = Path::new(CHECKOUT);
    copy_tree(
        &checkout.join("src/herdr_inbox"),
        &root.join("src/herdr_inbox"),
    );
    fs::copy(
        checkout.join("herdr-plugin.toml"),
        root.join("herdr-plugin.toml"),
    )
    .unwrap();
    let pyproject = fs::read_to_string(checkout.join("pyproject.toml")).unwrap();
    let own = format!("version = \"{}\"", env!("CARGO_PKG_VERSION"));
    assert!(pyproject.contains(&own), "pyproject.toml names {own}");
    let pyproject = pyproject.replacen(&own, &format!("version = \"{version}\""), 1);
    fs::write(root.join("pyproject.toml"), pyproject).unwrap();
    if revision != 1 {
        let store = root.join("src/herdr_inbox/store.py");
        let text = fs::read_to_string(&store).unwrap();
        let (rev, cols) = (
            (
                "\nMIGRATION_REVISION = 1\n",
                format!("\nMIGRATION_REVISION = {revision}\n"),
            ),
            (
                "\"remote_outbox\": {\"sending_at\": \"TEXT\"}",
                "\"remote_outbox\": {\"sending_at\": \"TEXT\", \"added_in_r2\": \"TEXT\"}"
                    .to_string(),
            ),
        );
        assert_eq!(text.matches(rev.0).count(), 1, "store.py's revision line");
        assert_eq!(text.matches(cols.0).count(), 1, "store.py's added columns");
        let text = text.replacen(rev.0, &rev.1, 1).replacen(cols.0, &cols.1, 1);
        fs::write(&store, text).unwrap();
    }
    root
}

/// `root/target/release/herdr-inbox`: this executable, which runs as a
/// Rust worker on `worker`.
pub fn with_rust_worker(root: &Path) {
    let exe = std::env::current_exe().unwrap();
    let link = root.join("target/release/herdr-inbox");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    symlink(exe, link).unwrap();
}

/// `root/bin/herdr-inbox`: a release binary from before P5, with no
/// `worker` (prints its version and exits 0).
pub fn with_hooks_only_binary(root: &Path) {
    let bin = root.join("bin/herdr-inbox");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    fs::write(&bin, "#!/bin/sh\necho 'herdr-inbox 0.10.6'\n").unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
}
