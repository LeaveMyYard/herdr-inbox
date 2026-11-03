//! The renderer against the app-screen goldens (tests/fixtures/screens/
//! {popup,popup-narrow}.json, captured from the Python popup by
//! tools/capture_goldens.py): the same data and the same keys give the same
//! screens, cell for cell (char, fg, bg, bold). The empty screen is the
//! exception: the port draws design 2C there (Q179).

mod screens;

use std::collections::BTreeMap;

use inbox_core::empty_inbox;
use inbox_core::popup::{Detail, QOption};
use ratatui_core::buffer::Buffer;
use screens::data::{BILLING, DOCS, SEARCH, alert, db, opts, populate, question, snapshot};
use screens::drive::{self, Pilot};
use screens::golden;
use serde_json::Value;

const POPUP: &str = include_str!("../../../tests/fixtures/screens/popup.json");
const NARROW: &str = include_str!("../../../tests/fixtures/screens/popup-narrow.json");

/// Every screen of the capture's scenarios, by golden file and name.
fn screens() -> BTreeMap<(&'static str, String), Buffer> {
    let mut out = BTreeMap::new();
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
    out
}

#[test]
fn every_golden_screen_is_drawn_cell_for_cell() {
    let shots = screens();
    let mut failures = Vec::new();
    let mut compared = 0;
    for (file, text) in [("popup", POPUP), ("popup-narrow", NARROW)] {
        let doc: Value = serde_json::from_str(text).unwrap();
        let size = &doc["size"];
        for (name, rows) in doc["screens"].as_object().unwrap() {
            if name == "empty" {
                continue; // design 2C replaces Python's "No open questions" (Q179)
            }
            let buf = shots
                .get(&(file, name.clone()))
                .unwrap_or_else(|| panic!("no scenario for {file}/{name}"));
            assert_eq!(
                serde_json::json!([buf.area.width, buf.area.height]),
                *size,
                "{file}/{name}"
            );
            compared += 1;
            if let Some(d) = golden::diff(
                &format!("{file}/{name}"),
                &golden::want(rows),
                &golden::got(buf),
            ) {
                failures.push(d);
            }
        }
    }
    assert_eq!(
        compared, 24,
        "every golden screen but the empty one is compared"
    );
    assert!(
        failures.is_empty(),
        "{} of {compared} screens differ:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn the_empty_popup_is_design_2c_centred() {
    let shots = screens();
    let buf = &shots[&("popup", "empty".to_string())];
    // the pilot draws one step after the popup opened on the empty view
    let frame = empty_inbox::frame_at(drive::STEP as f64 / 1000.0);
    assert_ne!(frame, empty_inbox::frame(0), "the scene has moved on");
    let (x0, y0) = (
        (110 - empty_inbox::W as u16) / 2,
        (30 - empty_inbox::H as u16) / 2,
    );
    let mut cells = 0;
    for (y, row) in golden::got(buf).iter().enumerate() {
        for (x, got) in row.iter().enumerate() {
            let inside = (x0..x0 + empty_inbox::W as u16).contains(&(x as u16))
                && (y0..y0 + empty_inbox::H as u16).contains(&(y as u16));
            let want = if inside {
                let c = frame.cell(x - usize::from(x0), y - usize::from(y0));
                (
                    c.ch.to_string(),
                    Some(c.fg.to_string()),
                    Some(c.bg.to_string()),
                    c.bold,
                )
            } else {
                (" ".to_string(), None, Some("#121212".to_string()), false)
            };
            assert_eq!(*got, want, "({x},{y})");
            cells += usize::from(inside);
        }
    }
    assert_eq!(cells, empty_inbox::W * empty_inbox::H);
}
