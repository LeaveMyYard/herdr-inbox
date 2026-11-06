//! A question from a herdr-core terminal, as a herdr server's worker meets
//! it (plan herdr-app-inbox §3.2): no pane, another "server", so nothing of a
//! tick claims, types, settles or notifies it. It is the core worker's
//! (`delivery::terminal`'s tests). The same for Python is pinned in tests/test_core_rows.py.

use herdr_inbox::clock::now_minus;
use herdr_inbox::store::hosts::{CORE_SERVER, CoreSeat};
use herdr_inbox::store::questions::NewQuestion;
use inbox_core::deliver::MAX_STALE_RETRIES;
use rusqlite::params;
use std::time::Duration;

use crate::{A, World};

impl World {
    /// A core question as `ask_operator` files it.
    fn core_question(&mut self) -> i64 {
        let q = self
            .store
            .insert(&NewQuestion {
                question: "Question Title 123",
                session_id: Some(A),
                cwd: Some("/r"),
                ..NewQuestion::default()
            })
            .unwrap();
        self.store.set_server(q.id, Some(CORE_SERVER)).unwrap();
        let seat = CoreSeat {
            terminal: "t3".into(),
            machine: None,
        };
        self.store.set_question_host(q.id, &seat).unwrap();
        q.id
    }

    /// Every column of `qid`'s rows, as text.
    fn core_rows(&self, qid: i64) -> Vec<String> {
        let mut out = Vec::new();
        for table in ["questions", "question_ext", "question_host"] {
            let mut stmt = self
                .store
                .db()
                .prepare(&format!("SELECT * FROM {table} WHERE id = ?"))
                .unwrap();
            let n = stmt.column_count();
            let mut rows = stmt.query(params![qid]).unwrap();
            while let Some(r) = rows.next().unwrap() {
                for i in 0..n {
                    out.push(format!("{:?}", r.get_ref(i).unwrap()));
                }
            }
        }
        out
    }
}

#[test]
fn a_tick_leaves_an_answered_core_question_as_it_is() {
    let mut w = World::new();
    let qid = w.core_question();
    w.answer(qid, "Yes");
    let long_ago = now_minus(Duration::from_secs(3600));
    w.store
        .db()
        .execute(
            "UPDATE questions SET answered_at = ? WHERE id = ?",
            params![long_ago, qid],
        )
        .unwrap();
    let before = w.core_rows(qid);
    w.fake.reset();
    for _ in 0..MAX_STALE_RETRIES + 3 {
        w.tick();
    }
    assert_eq!(w.core_rows(qid), before);
    assert_eq!(w.fake.agent_gets(), 0);
    assert!(w.notes.is_empty());
}
