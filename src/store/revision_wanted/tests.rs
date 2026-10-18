use super::*;

#[test]
fn the_marker_is_pythons_file_and_format() {
    // Python revision_wanted: "<db>.revision-wanted", "<revision> <int unix seconds>\n"
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    want(&db, 2, 1000.7);
    assert_eq!(
        fs::read_to_string(dir.path().join("inbox.db.revision-wanted")).unwrap(),
        "2 1000\n"
    );
    // a line Python wrote reads the same
    fs::write(path_for(&db), "3 1000\n").unwrap();
    assert_eq!(wanted(&db, 1001.0), Some(3));
}

#[test]
fn never_lowered_never_extended_and_it_expires() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    want(&db, 3, 1000.0);
    want(&db, 2, 1001.0);
    assert_eq!(wanted(&db, 1002.0), Some(3));
    assert_eq!(
        wanted(&db, 1000.0 + REVISION_WANTED_TTL + 1.0),
        None,
        "stale (not extended by the lower write)"
    );
    want(&db, 4, 5000.0);
    assert_eq!(
        wanted(&db, 5000.0 - CLOCK_SKEW - 1.0),
        None,
        "dated in the future"
    );
    fs::write(path_for(&db), "garbage").unwrap();
    assert_eq!(wanted(&db, 0.0), None);
    assert_eq!(wanted(&dir.path().join("other.db"), 1000.0), None);
}
