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

#[test]
fn old_markers_are_pruned_and_fresh_ones_kept() {
    let dir = tempfile::tempdir().unwrap();
    record_start(dir.path(), SID, Some("w1:p2")).unwrap();
    let other = "bbbbbbbb-0000-4000-8000-00000000000b";
    record_start(dir.path(), other, None).unwrap();
    let old = dir.path().join("starts").join(SID.to_lowercase());
    let month_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(31 * 86_400);
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(month_ago)
        .unwrap();
    assert_eq!(
        prune_starts(dir.path(), std::time::Duration::from_secs(30 * 86_400)),
        1
    );
    assert!(!old.exists() && dir.path().join("starts").join(other).exists());
    assert_eq!(
        prune_starts(&dir.path().join("none"), std::time::Duration::ZERO),
        0
    );
}
