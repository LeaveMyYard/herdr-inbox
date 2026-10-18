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

#[test]
fn the_shared_vectors_read_as_in_python() {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/vectors/revision_wanted.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    for case in doc["cases"].as_array().unwrap() {
        fs::write(path_for(&db), case["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            wanted(&db, case["now"].as_f64().unwrap()),
            case["wanted"].as_i64(),
            "{case}"
        );
    }
}

#[test]
fn a_held_marker_lock_never_holds_the_hook() {
    // #75 final pass L1: another writer holding the lock costs at most WANT_LOCK_WAIT
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("inbox.db");
    let mut lock = path_for(&db).into_os_string();
    lock.push(".lock");
    let _held = LockFile::new(PathBuf::from(lock)).exclusive().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let db2 = db.clone();
    std::thread::spawn(move || {
        want(&db2, 2, 1000.0);
        tx.send(()).unwrap();
    });
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .expect("want() returned while the lock was held");
    assert!(!path_for(&db).exists(), "nothing written without the lock");
}
