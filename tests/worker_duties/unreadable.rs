//! An input box that stays unreadable says why, once (Python
//! tests/test_composer_draft.py `test_l2_*`), and nothing is typed meanwhile.

use herdr_inbox::delivery::run::Unreadable;

use crate::World;

const NOTICE: &str = "cannot read the agent's input box";

impl World {
    /// Asked and answered with `w1:p1`'s input box unreadable.
    fn answered_unreadable(&mut self) -> i64 {
        self.fake.set(|k| k.ansi = Some(String::new()));
        self.pending_answer("idle")
    }
}

#[test]
fn an_unreadable_input_box_says_why_once() {
    let mut w = World::new();
    let qid = w.answered_unreadable();
    for _ in 0..3 {
        w.tick();
    }
    let notes = w.shown(NOTICE);
    assert_eq!(notes.len(), 1, "{:?}", w.notes);
    assert!(notes[0].1.contains(&format!("Q{qid}")), "{notes:?}");
    assert!(w.fake.prompts().is_empty());

    w.fake.set(|k| k.ansi = None); // readable again: the next stall is told anew
    w.tick();
    assert!(w.store.get(qid).unwrap().unwrap().delivered_at.is_some());
    w.answered_unreadable();
    assert_eq!(w.shown(NOTICE).len(), 2, "{:?}", w.notes);
}

#[test]
fn a_brief_unreadable_read_does_not_notify() {
    let mut w = World::new();
    w.unreadable = Unreadable::default();
    w.answered_unreadable();
    w.tick();
    assert!(w.shown(NOTICE).is_empty(), "{:?}", w.notes);
}
