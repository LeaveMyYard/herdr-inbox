//! The renderer against the app-screen goldens (tests/fixtures/screens/
//! {popup,popup-narrow}.json, captured from the Python popup by
//! tools/capture_goldens.py): the same data and the same keys give the same
//! screens, cell for cell (char, fg, bg, bold); popup-extra.json holds the
//! views the others do not reach, popup-remote.json a remote machine's rows
//! and views (P6-D2). The empty screen is the exception: the
//! port draws design 2C there (Q179).

mod screens;

use inbox_core::empty_inbox;
use screens::base::Shots;
use screens::drive;
use screens::golden;
use serde_json::Value;

const POPUP: &str = include_str!("../../../tests/fixtures/screens/popup.json");
const NARROW: &str = include_str!("../../../tests/fixtures/screens/popup-narrow.json");
const EXTRA: &str = include_str!("../../../tests/fixtures/screens/popup-extra.json");
const REMOTE: &str = include_str!("../../../tests/fixtures/screens/popup-remote.json");
const DEVIATIONS: &str = include_str!("../../../tests/fixtures/popup-deviations.json");

/// The rows where the Rust popup deliberately differs from Python (the
/// README lists why): `(file, screen, row, runs)`.
fn deviations() -> Vec<(String, String, usize, Value)> {
    let doc: Value = serde_json::from_str(DEVIATIONS).unwrap();
    let list = doc["screens"].as_array().unwrap().iter().map(|d| {
        let text = |k: &str| d[k].as_str().unwrap().to_string();
        let row = usize::try_from(d["row"].as_u64().unwrap()).unwrap();
        (text("file"), text("screen"), row, d["runs"].clone())
    });
    list.collect()
}

/// A golden screen as Rust must draw it.
fn with_deviations(file: &str, name: &str, rows: &Value) -> Value {
    let mut rows = rows.clone();
    for (_, _, y, runs) in deviations()
        .into_iter()
        .filter(|d| d.0 == file && d.1 == name)
    {
        rows[y] = runs;
    }
    rows
}

/// When Python is fixed and the golden recaptured, this asks to drop the
/// deviation.
#[test]
fn every_screen_deviation_still_differs_from_python() {
    let doc: Value = serde_json::from_str(REMOTE).unwrap();
    let list = deviations();
    assert!(!list.is_empty());
    for (file, name, y, runs) in list {
        assert_eq!(file, "popup-remote", "only the remote screens deviate");
        let python = &doc["screens"][&name][y];
        assert!(python.is_array(), "no row {y} in {name}");
        assert_ne!(*python, runs, "{name}: Python draws this now");
    }
}

/// Every scenario's screens.
fn screens() -> Shots {
    let mut out = Shots::new();
    screens::base::screens(&mut out);
    screens::extra::screens(&mut out);
    screens::remote::screens(&mut out);
    out
}

#[test]
fn every_golden_screen_is_drawn_cell_for_cell() {
    let shots = screens();
    let mut failures = Vec::new();
    let mut compared = 0;
    for (file, text) in [
        ("popup", POPUP),
        ("popup-narrow", NARROW),
        ("popup-extra", EXTRA),
        ("popup-remote", REMOTE),
    ] {
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
                &golden::want(&with_deviations(file, name, rows)),
                &golden::got(buf),
            ) {
                failures.push(d);
            }
        }
    }
    assert_eq!(
        compared, 56,
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

/// The screen's rows as text.
fn rows(buf: &ratatui_core::buffer::Buffer) -> Vec<String> {
    golden::got(buf)
        .iter()
        .map(|r| r.iter().map(|c| c.0.as_str()).collect())
        .collect()
}

/// Python `test_tab_row_scrolls_to_keep_the_current_tab_visible`: at 40
/// columns the tab row scrolls so the current chip (reversed) is whole.
#[test]
fn the_tab_row_scrolls_to_keep_the_current_tab_in_view() {
    use screens::data::{BILLING, opts, question, snapshot};
    let four = (0..4)
        .map(|i| {
            question(
                i + 1,
                1,
                BILLING,
                &format!("Question number {i}?"),
                opts(&[]),
                &format!("Header {i:02}"),
            )
        })
        .collect();
    let mut p = drive::Pilot::open(snapshot(four, vec![], vec![]), (40, 20));
    p.press(&["right"; 4]);
    let buf = p.screen();
    let row = &golden::got(&buf)[3];
    let reversed: Vec<usize> = (0..row.len())
        .filter(|&x| row[x].2.as_deref() == Some("#e0e0e0"))
        .collect();
    let text: String = reversed.iter().map(|&x| row[x].0.as_str()).collect();
    assert_eq!(
        text,
        " Submit ",
        "the current chip, whole: {:?}",
        rows(&buf)[3]
    );
    assert!(
        !rows(&buf)[3].contains("│ Billing API"),
        "scrolled past the first chips"
    );
    assert!(
        reversed.iter().all(|&x| (2..38).contains(&x)),
        "inside the row"
    );
}

/// Python `test_preview_layout`: the focused option's preview beside the
/// options, "No preview available" for one without.
#[test]
fn a_preview_shows_beside_the_options() {
    use inbox_core::popup::QOption;
    use screens::data::{BILLING, question, snapshot};
    let small = QOption {
        preview: Some("workers = 2\nmemory = 1Gi".into()),
        ..QOption::label("Small")
    };
    let large = QOption {
        description: Some("x".into()),
        ..QOption::label("Large")
    };
    let q = question(1, 1, BILLING, "Which config?", vec![small, large], "");
    let mut p = drive::Pilot::open(snapshot(vec![q], vec![], vec![]), (100, 30));
    let first = rows(&p.screen());
    let at = |rows: &[String], s: &str| {
        rows.iter()
            .find_map(|r| r.find(s).map(|i| r[..i].chars().count()))
    };
    let (rule, preview) = (at(&first, "│").unwrap(), at(&first, "workers = 2").unwrap());
    assert!(at(&first, "Small").unwrap() < rule && rule < preview);
    p.press(&["down"]);
    let second = rows(&p.screen());
    assert!(at(&second, "No preview available").is_some() && at(&second, "workers").is_none());
}

/// Python's `#radar-hint` and `#held-hint`: under the view, in that order,
/// the view (and its own hint) shortened by them.
#[test]
fn the_radar_and_deep_link_hints_sit_under_the_view() {
    use inbox_core::popup::{DeepLink, Entry, Open, Popup};
    use inbox_core::popup_view::{Ctx, ViewState, render};
    use ratatui_core::{buffer::Buffer, layout::Rect};
    let link = DeepLink {
        target: Some(Entry::Group(4)),
        qid: Some(4),
        hold: true,
        missing: String::new(),
    };
    let snapshot = screens::data::snapshot(screens::data::populate(), vec![], vec![]);
    let open = Open {
        focus_pane: Some("w1:p1".into()),
        snapshot,
        deeplink: Some(link),
    };
    let (popup, _) = Popup::open(open, 1_000);
    assert!(popup.held());
    let ctx = Ctx {
        now: 2_000,
        wall: screens::data::HELD,
        radar_missing: true,
        ..Ctx::default()
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 110, 30));
    render(&popup, &ctx, &mut ViewState::default(), &mut buf);
    let rows = rows(&buf);
    assert!(
        rows[26].trim_start().starts_with("Enter to select"),
        "{:?}",
        rows[26]
    );
    assert!(
        rows[27]
            .trim_start()
            .starts_with("herdr-radar is installed"),
        "{:?}",
        rows[27]
    );
    assert!(
        rows[28]
            .trim_start()
            .starts_with("Opened from another plugin"),
        "{:?}",
        rows[28]
    );
    let cell = &buf[(2, 28)];
    assert_eq!(cell.fg, ratatui_core::style::Color::Rgb(0xb1, 0xb9, 0xf9));
    assert!(cell.modifier.contains(ratatui_core::style::Modifier::BOLD));
}

/// A note is italic, as Python draws it (the goldens
/// record only bold).
#[test]
fn a_note_is_italic() {
    use ratatui_core::style::Modifier;
    let shots = screens();
    let italic = |file: &'static str, name: &str, text: &str| {
        let buf = &shots[&(file, name.to_string())];
        let rows = rows(buf);
        let (y, row) = rows
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(text))
            .unwrap();
        let x = row[..row.find(text).unwrap()].chars().count();
        (0..text.chars().count()).all(|i| {
            buf[(x as u16 + i as u16, y as u16)]
                .modifier
                .contains(Modifier::ITALIC)
        })
    };
    assert!(italic(
        "popup-extra",
        "a note under SQLite",
        "keep it small"
    ));
    assert!(!italic("popup-extra", "a note under SQLite", "Notes:"));
}

/// Under the empty view (design 2C) the radar hint still shows, as Python's
/// `#radar-hint` does under its "No open questions".
#[test]
fn the_empty_popup_keeps_the_radar_hint() {
    use inbox_core::popup::{Open, Popup};
    use inbox_core::popup_view::{Ctx, ViewState, render};
    use ratatui_core::{buffer::Buffer, layout::Rect};
    let (popup, _) = Popup::open(Open::default(), 1_000);
    let ctx = Ctx {
        now: 1_000,
        radar_missing: true,
        ..Ctx::default()
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 110, 30));
    render(&popup, &ctx, &mut ViewState::default(), &mut buf);
    let rows = rows(&buf);
    assert!(
        rows[28]
            .trim_start()
            .starts_with("herdr-radar is installed"),
        "{:?}",
        rows[28]
    );
    assert!(
        rows.iter().any(|r| r.contains("No open questions")),
        "the scene is still there"
    );
    assert_eq!(
        buf[(2, 28)].fg,
        ratatui_core::style::Color::Rgb(0xa0, 0xa0, 0xa0)
    );
}

/// Python `test_remote_text_is_escaped`: a remote machine's text is drawn as
/// written, markup and all (the renderer has no markup to inject into).
#[test]
fn remote_text_is_drawn_as_written() {
    use inbox_core::popup::Machine;
    use screens::data::{SEARCH, opts, question, snapshot};
    use screens::remote::{group, rq};
    let local = question(1, 1, SEARCH, "Local?", opts(&[]), "");
    let mut s = snapshot(vec![local], vec![], vec![]);
    let q = rq("m1", 1, 1, "[b]bold[/b] [link=x]x[/link]?", opts(&[]));
    s.remote.mirror.push(q.r#ref());
    s.remote.groups.push(group(vec![q]));
    s.remote.machines.push(Machine {
        machine_id: "m1".into(),
        label: Some("project1".into()),
        offline: None,
    });
    let mut p = drive::Pilot::open(s, (110, 30));
    let rows = rows(&p.screen());
    assert!(
        rows.iter()
            .any(|r| r.contains("project1:Q1 [b]bold[/b] [link=x]x[/link]? · 1 question")),
        "{rows:#?}"
    );
}
