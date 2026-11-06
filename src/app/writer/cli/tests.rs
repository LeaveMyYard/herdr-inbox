use std::ffi::OsString;
use std::path::PathBuf;

use super::{HELP, Parsed, app_home, parse};

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_string()).collect()
}

#[test]
fn help_starts_with_the_line_a_probe_reads() {
    assert_eq!(HELP.lines().next(), Some("herdr-inbox app-feed v1"));
    assert_eq!(parse(&args(&["--help"])), Some(Parsed::Help));
}

#[test]
fn ensure_takes_an_absolute_home() {
    assert_eq!(parse(&args(&["--ensure"])), Some(Parsed::Ensure(None)));
    assert_eq!(
        parse(&args(&["--app-home", "/a/b", "--ensure"])),
        Some(Parsed::Ensure(Some(PathBuf::from("/a/b"))))
    );
}

#[test]
fn a_writer_is_told_its_binarys_hash() {
    assert_eq!(
        parse(&args(&["--writer", "--binary-sha256", "x"])),
        Some(Parsed::Writer(None, "x".to_string()))
    );
}

#[test]
fn anything_else_is_bad_arguments() {
    for bad in [
        &[][..],
        &["--ensure", "--ensure"],
        &["--ensure", "--writer"],
        &["--ensure", "--app-home"],
        &["--ensure", "--app-home", "relative"],
        &["--ensure", "--app-home", "/a", "--app-home", "/b"],
        &["--ensure", "--help"],
        &["--app-home", "/a"],
        &["ensure"],
        &["--writer"],
        &["--ensure", "--binary-sha256", "x"],
    ] {
        assert_eq!(parse(&args(bad)), None, "{bad:?}");
    }
}

#[test]
fn the_app_home_is_the_apps_own() {
    let vars = |name: &str| -> Option<OsString> {
        match name {
            "HERDR_APP_HOME" => Some("/x/app".into()),
            "HOME" => Some("/x/me".into()),
            _ => None,
        }
    };
    assert_eq!(app_home(vars), Some(PathBuf::from("/x/app")));
    let home_only = |name: &str| (name == "HOME").then(|| OsString::from("/x/me"));
    assert_eq!(
        app_home(home_only),
        Some(PathBuf::from("/x/me/.config/herdr-app"))
    );
    assert_eq!(app_home(|_| None), None);
    let relative = |name: &str| (name == "HERDR_APP_HOME").then(|| OsString::from("rel"));
    assert_eq!(app_home(relative), None);
}
