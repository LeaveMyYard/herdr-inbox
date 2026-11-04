use super::*;

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn a_request_is_taken_once_by_its_popup() {
    let d = dir();
    let nonce = write_request(d.path(), "question", "12", true, 100.0).unwrap();
    assert_eq!(nonce.len(), 32);
    let names: Vec<_> = fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec!["open-request.json"], "no temp file left");
    assert_eq!(
        consume_request(d.path(), None, 101.0),
        None,
        "a popup opened by hand"
    );
    assert_eq!(consume_request(d.path(), Some("other"), 101.0), None);
    assert!(
        request_path(d.path()).exists(),
        "another popup's request stays"
    );
    let got = consume_request(d.path(), Some(&nonce), 101.0).unwrap();
    assert_eq!(
        (got.kind.as_str(), got.value.as_str(), got.hold),
        ("question", "12", true)
    );
    assert!(!request_path(d.path()).exists());
    assert_eq!(consume_request(d.path(), Some(&nonce), 101.0), None, "once");
    assert!(
        fs::read_dir(d.path()).unwrap().next().is_none(),
        "the taken file is gone"
    );
}

#[test]
fn the_newest_request_wins_and_a_stale_one_is_dropped() {
    let d = dir();
    let first = write_request(d.path(), "group", "1", true, 100.0).unwrap();
    let second = write_request(d.path(), "group", "2", false, 100.0).unwrap();
    assert_eq!(consume_request(d.path(), Some(&first), 100.0), None);
    let got = consume_request(d.path(), Some(&second), 104.0).unwrap();
    assert_eq!((got.value.as_str(), got.hold), ("2", false));
    let third = write_request(d.path(), "group", "3", true, 100.0).unwrap();
    assert_eq!(
        consume_request(d.path(), Some(&third), 105.5),
        None,
        "older than REQUEST_TTL"
    );
    assert!(!request_path(d.path()).exists(), "taken and dropped");
}

#[test]
fn an_invalid_target_is_never_written() {
    let d = dir();
    assert_eq!(write_request(d.path(), "question", "QQ1", true, 1.0), None);
    assert!(!request_path(d.path()).exists());
}
