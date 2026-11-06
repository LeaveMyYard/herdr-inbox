use std::path::{Path, PathBuf};

use super::path;

fn run() -> Option<&'static Path> {
    Some(Path::new("/fixture/run"))
}

#[test]
fn the_runtime_directory_and_the_session_name_it() {
    assert_eq!(
        path(run(), None, 7, true),
        Some(PathBuf::from("/fixture/run/herdr-core/default.sock"))
    );
    assert_eq!(
        path(run(), Some("x1"), 7, false),
        Some(PathBuf::from("/fixture/run/herdr-core/x1.sock"))
    );
}

#[test]
fn with_no_runtime_directory_only_linux_has_a_place() {
    assert_eq!(
        path(None, None, 7, true),
        Some(PathBuf::from("/run/user/7/herdr-core/default.sock"))
    );
    assert_eq!(path(None, None, 7, false), None);
}

#[test]
fn a_relative_directory_or_an_odd_session_names_nothing() {
    assert_eq!(path(Some(Path::new("run")), None, 7, true), None);
    for session in ["", "../x", "a/b", ".hidden"] {
        assert_eq!(path(run(), Some(session), 7, true), None, "{session}");
    }
}
