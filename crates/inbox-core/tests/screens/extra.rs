//! The scenarios of `tools/capture_goldens.py`'s `extra_screens`: the views
//! the main scenarios do not reach.

use inbox_core::popup::{AGENT_GONE, Alert};

use super::base::Shots;
use super::data::{BILLING, DOCS, SEARCH, alert, opts, populate, question, snapshot};
use super::drive::Pilot;

const FILE: &str = "popup-extra";

pub fn screens(out: &mut Shots) {
    let size = (110, 30);
    let mut shot = |name: &str, p: &mut Pilot| {
        out.insert((FILE, name.to_string()), p.screen());
    };

    let mut qs = populate();
    let late = [
        (5, DOCS, "Bump the timeout?"),
        (6, SEARCH, "Rename the index?"),
        (7, SEARCH, "Drop the cache?"),
        (8, BILLING, "Retry the export?"),
    ];
    let errors = [
        "pending: blocked: prompt".to_string(),
        format!("{AGENT_GONE}pane w1:p2 closed"),
        format!("{AGENT_GONE}pane w1:p2 closed"),
        "pending: requeued: earlier miss".to_string(),
    ];
    for ((id, who, text), err) in late.into_iter().zip(errors) {
        let mut q = question(id, id, who, text, opts(&["yes", "no"]), "H");
        (q.status, q.answer, q.delivery_error) = ("answered".into(), Some("yes".into()), Some(err));
        qs.push(q);
    }
    let mut p = Pilot::open(snapshot(qs, vec![5, 6, 7, 8], vec![]), size);
    shot("piles", &mut p);
    p.press(&["down", "down", "down", "down"]);
    shot("piles, cursor on a requeued answer", &mut p);
    p.press(&["enter"]);
    shot("requeued answer opened", &mut p);
    p.press(&["escape", "down"]);
    shot("piles, cursor on the gone summary", &mut p);
    p.press(&["up", "up", "up", "d"]);
    shot("group dismiss from the list", &mut p);

    let mut p = Pilot::open(snapshot(populate(), vec![], vec![]), size);
    p.press(&["enter", "down", "n"]);
    p.type_text("keep it small");
    p.press(&["enter"]);
    shot("a note under SQLite", &mut p);
    p.press(&["n"]);
    shot("the note box reopened on its note", &mut p);
    p.press(&["escape", "down", "n"]);
    shot("an empty note box", &mut p);
    p.press(&["escape", "4", "enter"]);
    p.type_text("Firebird");
    p.press(&["enter"]);
    shot("typed answer drafted, next tab", &mut p);
    p.press(&["space", "tab"]);
    shot("review with a typed draft", &mut p);
    p.press(&["shift+tab"]);
    let mut now = p.popup.data().clone();
    if let Some(q) = now.questions.iter_mut().find(|q| q.id == 3) {
        q.status = "withdrawn".into();
    }
    now.open.retain(|&id| id != 3);
    p.poll(now);
    shot("the tab closed under the operator", &mut p);

    let blocked = Alert {
        kind: "blocked".into(),
        message: None,
        ..alert(1, SEARCH, "Approval prompt", "", "warning", "claude")
    };
    let callers = [BILLING, SEARCH, DOCS];
    let qs = (0..14)
        .map(|i| {
            let text = format!("Question {i} with a fairly long text that goes on?");
            question(
                i + 1,
                i + 1,
                callers[i as usize % 3],
                &text,
                opts(&["a", "b"]),
                "",
            )
        })
        .collect();
    let mut p = Pilot::open(snapshot(qs, vec![], vec![blocked]), size);
    shot("a long list", &mut p);
    p.press(&["down"; 13]);
    shot("a long list, scrolled", &mut p);
    let mut keys = vec!["up"; 20];
    keys.push("enter");
    p.press(&keys);
    shot("blocked alert opened", &mut p);

    let mut p = Pilot::open(snapshot(populate(), vec![], vec![]), size);
    p.press(&["tab", "tab", "enter", "n"]);
    p.type_text("abc");
    p.press(&["tab"]);
    shot("a note box blurred by Tab", &mut p);
    p.press(&["4", "enter"]);
    p.type_text("xyz");
    p.press(&["tab"]);
    shot("a typed box blurred, the note box gone", &mut p);

    let qs = (0..30)
        .map(|i| {
            question(
                i + 1,
                i + 1,
                callers[i as usize % 3],
                &format!("Row {i}?"),
                opts(&["a", "b"]),
                "",
            )
        })
        .collect();
    let mut p = Pilot::open(snapshot(qs, vec![], vec![]), size);
    p.press(&["down"; 25]);
    shot("a longer list, down to row 26", &mut p);
    p.press(&["up"; 12]);
    shot("a longer list, back up to row 14", &mut p);
}
