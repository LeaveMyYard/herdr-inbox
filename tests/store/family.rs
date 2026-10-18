use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use herdr_inbox::store::family::{Continuations, FAMILY_LIMIT, family};
use rusqlite::Connection;

use crate::common::*;

/// Transcripts as a map: session -> the session its transcript continues in.
#[derive(Default)]
pub struct Fake {
    pub next: HashMap<String, String>,
    pub asked: RefCell<Vec<(String, Option<String>)>>,
}

impl Continuations for Fake {
    fn find(&self, session_id: &str, known: Option<&str>) -> Option<PathBuf> {
        self.asked
            .borrow_mut()
            .push((session_id.to_owned(), known.map(str::to_owned)));
        Some(PathBuf::from(session_id))
    }
    fn continued_in(&self, path: &Path) -> Option<String> {
        self.next.get(path.to_str()?).cloned()
    }
}

fn nominate(db: &Connection, old: &str, new: &str) {
    db.execute(
        "INSERT INTO sessions (session_id, continued_in) VALUES (?, ?)",
        [old, new],
    )
    .unwrap();
}

fn sid(n: usize) -> String {
    format!("aaaaaaaa-0000-4000-8000-{n:012}")
}

#[test]
fn a_session_and_its_live_predecessors_transitively() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let (p1, p2, stale) = (sid(1), sid(2), sid(3));
    nominate(&db, &p1, A);
    nominate(&db, &p2, &p1);
    nominate(&db, &stale, A); // resumed since: its transcript no longer continues into A
    db.execute(
        "UPDATE sessions SET transcript_path = '/t/p1.jsonl' WHERE session_id = ?",
        [&p1],
    )
    .unwrap();
    let mut fake = Fake::default();
    fake.next.insert(p1.clone(), A.to_owned());
    fake.next.insert(p2.clone(), p1.clone());
    assert_eq!(
        family(&db, A, &fake).unwrap(),
        [A.to_owned(), p1.clone(), p2]
    );
    assert!(
        fake.asked
            .borrow()
            .contains(&(p1, Some("/t/p1.jsonl".to_owned()))),
        "the recorded path is used"
    );
    assert_eq!(family(&db, "", &fake).unwrap(), Vec::<String>::new());
    assert_eq!(family(&db, B, &fake).unwrap(), [B.to_owned()]);
}

#[test]
fn successors_and_cycles_never_join_and_the_family_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&migrated(dir.path()));
    let mut fake = Fake::default();
    nominate(&db, A, B); // A continued into B: B is A's successor, never in A's family
    fake.next.insert(A.to_owned(), B.to_owned());
    assert_eq!(family(&db, A, &fake).unwrap(), [A.to_owned()]);
    nominate(&db, B, A); // and a cycle back
    fake.next.insert(B.to_owned(), A.to_owned());
    assert_eq!(family(&db, A, &fake).unwrap(), [A.to_owned(), B.to_owned()]);
    // a long chain stops at the limit
    for n in 1..100 {
        nominate(&db, &sid(n + 100), &sid(n + 99));
        fake.next.insert(sid(n + 100), sid(n + 99));
    }
    assert_eq!(family(&db, &sid(100), &fake).unwrap().len(), FAMILY_LIMIT);
}
