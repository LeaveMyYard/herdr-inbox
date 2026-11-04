//! The scenarios of `tools/capture_goldens.py`'s `popup_screens`: the
//! demo data, the keys, the screens they reach.

use std::collections::BTreeMap;

use inbox_core::popup::{Detail, QOption};
use ratatui_core::buffer::Buffer;

use super::data::{BILLING, DOCS, SEARCH, alert, db, opts, populate, question, snapshot};
use super::drive::Pilot;

/// The screens, by golden file and name.
pub type Shots = BTreeMap<(&'static str, String), Buffer>;

/// Every screen of the capture's scenarios, by golden file and name.
pub fn screens(out: &mut Shots) {
    let size = (110, 30);
    let mut shot = |file: &'static str, name: &str, p: &mut Pilot| {
        out.insert((file, name.to_string()), p.screen());
    };

    let mut p = Pilot::open(snapshot(vec![], vec![], vec![]), size);
    shot("popup", "empty", &mut p);

    let mut qs = populate();
    let mut late = question(
        5,
        5,
        DOCS,
        "Bump the docs build timeout to 90 s?",
        opts(&["yes", "no"]),
        "Timeout",
    );
    (late.status, late.answer) = ("answered".into(), Some("yes".into()));
    late.delivery_error = Some("no live agent in the pane".into());
    qs.push(late);
    qs[1].draft = Some(Detail {
        choices: vec!["SQLite".into()],
        ..Detail::default()
    });
    let mut p = Pilot::open(snapshot(qs, vec![5], vec![]), size);
    shot("popup", "list", &mut p);
    p.press(&["down"]);
    shot("popup", "list, cursor on the second row", &mut p);
    p.press(&["up", "enter", "left"]);
    shot("popup", "group: DB choice tab (drafted)", &mut p);
    p.press(&["right"]);
    shot("popup", "group: Cache tab (multi-select)", &mut p);
    p.press(&["space", "down", "space"]);
    shot("popup", "group: Cache, two picked", &mut p);
    p.press(&["tab"]);
    shot("popup", "group: Submit tab", &mut p);
    p.press(&["shift+tab", "shift+tab", "n"]);
    p.type_text("no server to run");
    shot("popup", "group: note box open", &mut p);
    p.press(&["escape", "d"]);
    shot("popup", "dismiss confirmation", &mut p);
    p.press(&["escape", "escape"]);
    shot("popup", "back on the list", &mut p);
    p.press(&["down", "down", "down"]);
    shot("popup", "list, cursor on the ⚠ entry", &mut p);
    p.press(&["enter"]);
    shot("popup", "⚠ entry opened", &mut p);

    let mut q = question(
        1,
        1,
        BILLING,
        "Which database should the invoice store use?",
        db(),
        "DB choice",
    );
    (q.context, q.asked_by) = (Some("Invoice rows ~1M/day".into()), Some("planner".into()));
    let mut p = Pilot::open(snapshot(vec![q], vec![], vec![]), size);
    shot("popup", "single", &mut p);
    p.press(&["down"]);
    shot("popup", "single, cursor on SQLite", &mut p);
    p.press(&["4", "enter"]);
    p.type_text("MariaDB");
    shot("popup", "single, typing something else", &mut p);

    let preview = |label: &str, desc: &str, n: u8, mem: &str, cpu: &str| QOption {
        label: label.into(),
        description: Some(desc.into()),
        preview: Some(format!(
            "[worker]\nthreads = {n}\nmemory = \"{mem}\"\ncpu = \"{cpu}\""
        )),
    };
    let options = vec![
        preview("Small", "2 workers", 2, "1Gi", "500m"),
        preview("Large", "8 workers", 8, "4Gi", "2"),
        QOption::label("Custom"),
    ];
    let q = question(
        1,
        1,
        BILLING,
        "Which worker profile for the invoice exporter?",
        options,
        "Profile",
    );
    let mut p = Pilot::open(snapshot(vec![q], vec![], vec![]), size);
    shot("popup", "preview", &mut p);
    p.press(&["down", "down"]);
    shot("popup", "preview, no preview for Custom", &mut p);

    let a = alert(
        1,
        SEARCH,
        "Deploy needs your eyes",
        "prod rollout paused at 50%",
        "urgent",
        "deployer",
    );
    let q1 = question(
        1,
        1,
        DOCS,
        "Publish the v2 docs now or after the release?",
        opts(&["now", "after"]),
        "Rollout",
    );
    let q2 = question(
        2,
        2,
        SEARCH,
        "Keep the old query syntax as an alias?",
        opts(&["yes", "no"]),
        "Syntax",
    );
    let mut p = Pilot::open(snapshot(vec![q1, q2], vec![], vec![a]), size);
    shot("popup", "alert and questions", &mut p);
    p.press(&["up", "enter"]);
    shot("popup", "alert opened", &mut p);

    let mut p = Pilot::open(snapshot(populate(), vec![], vec![]), size);
    shot("popup", "before arrivals", &mut p);
    let mut qs = populate();
    qs.push(question(
        5,
        5,
        SEARCH,
        "Keep the old query syntax as an alias?",
        opts(&["yes", "no"]),
        "Syntax",
    ));
    qs.push(question(
        6,
        6,
        DOCS,
        "Which landing page copy do you prefer?",
        opts(&["A", "B"]),
        "Copy",
    ));
    p.poll(snapshot(qs.clone(), vec![], vec![]));
    shot("popup", "after one poll", &mut p);
    qs[0].status = "withdrawn".into();
    p.poll(snapshot(qs, vec![], vec![]));
    shot("popup", "a row withdrawn by its agent", &mut p);

    let narrow = (40, 20);
    let four: Vec<_> = (1..=4)
        .map(|i| {
            let mut q = question(
                i,
                1,
                BILLING,
                &format!("Question number {i}?"),
                opts(&["yes", "no"]),
                &format!("Head{i}"),
            );
            q.group_title = Some("Four at once".into());
            q
        })
        .collect();
    let mut p = Pilot::open(snapshot(four, vec![], vec![]), narrow);
    shot("popup-narrow", "first tab", &mut p);
    p.press(&["right", "right", "right"]);
    shot("popup-narrow", "fourth tab", &mut p);
    p.press(&["right"]);
    shot("popup-narrow", "submit tab", &mut p);
}
