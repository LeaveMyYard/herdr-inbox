use std::fs;

use super::*;

const SID: &str = "AAAAAAAA-0000-4000-8000-000000000001";

#[test]
fn a_marker_holds_the_pane_under_the_canonical_id() {
    let dir = tempfile::tempdir().unwrap();
    record_start(dir.path(), SID, Some("w1:p2")).unwrap();
    let marker = dir.path().join("starts").join(SID.to_lowercase());
    assert_eq!(fs::read_to_string(&marker).unwrap(), "w1:p2");
    record_start(dir.path(), SID, None).unwrap();
    assert_eq!(fs::read_to_string(&marker).unwrap(), "");
    let names: Vec<_> = fs::read_dir(dir.path().join("starts"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        [std::ffi::OsString::from(SID.to_lowercase())],
        "no temporary file is left"
    );
}

#[test]
fn an_id_that_is_not_a_session_id_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for bad in [
        "",
        "s",
        "../x",
        "*",
        "aaaaaaaa-0000-4000-8000-00000000000a\n",
    ] {
        record_start(dir.path(), bad, Some("w1:p1")).unwrap();
    }
    assert!(!dir.path().join("starts").exists());
}
