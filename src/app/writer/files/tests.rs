use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::Paths;

fn paths() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    fs::create_dir_all(&paths.data).unwrap();
    fs::set_permissions(&paths.data, fs::Permissions::from_mode(0o700)).unwrap();
    (dir, paths)
}

#[test]
fn a_replace_leaves_a_private_file_and_no_staging() {
    let (_dir, paths) = paths();
    paths.replace("feed.json", ".feed.tmp", b"one").unwrap();
    paths.replace("feed.json", ".feed.tmp", b"two").unwrap();
    assert_eq!(fs::read(paths.file("feed.json")).unwrap(), b"two");
    assert!(!paths.file(".feed.tmp").exists());
    let mode = fs::metadata(paths.file("feed.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_leftover_staging_file_is_replaced_not_followed() {
    let (dir, paths) = paths();
    let elsewhere = dir.path().join("elsewhere");
    fs::write(&elsewhere, b"keep").unwrap();
    std::os::unix::fs::symlink(&elsewhere, paths.file(".feed.tmp")).unwrap();
    paths.replace("feed.json", ".feed.tmp", b"new").unwrap();
    assert_eq!(fs::read(&elsewhere).unwrap(), b"keep");
    assert_eq!(fs::read(paths.file("feed.json")).unwrap(), b"new");
}

#[test]
fn only_a_private_real_folder_will_do() {
    let (dir, paths) = paths();
    assert!(paths.check_folder().is_ok());
    fs::set_permissions(&paths.data, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(paths.check_folder().is_err(), "open to others");
    fs::set_permissions(&paths.data, fs::Permissions::from_mode(0o700)).unwrap();
    let real = dir.path().join("real");
    fs::rename(&paths.data, &real).unwrap();
    std::os::unix::fs::symlink(&real, &paths.data).unwrap();
    assert!(paths.check_folder().is_err(), "a link");
}
