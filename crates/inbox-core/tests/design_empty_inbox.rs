//! The empty inbox against design 2C (`tests/fixtures/design_empty_inbox.json`,
//! captured by node from the design's own `inboxC(f)` in herdr-panorama
//! `docs/design/herdr-tui.js` with `tools/capture_empty_inbox.mjs`): every
//! frame of the roll cell for cell (char, fg, bg and bold, blanks included),
//! the loop, the frame timing and the palette.

use inbox_core::empty_inbox::{
    FRAME_SECONDS, Grid, H, PERIOD, Rgb, W, frame, frame_at, index_at, palette,
};
use serde_json::Value;

const GOLDEN: &str = include_str!("../../../tests/fixtures/design_empty_inbox.json");

fn golden() -> Value {
    serde_json::from_str(GOLDEN).unwrap()
}

/// A cell as the fixture spells it: `(ch, "#rrggbb" fg, "#rrggbb" bg, bold)`.
type Shown = (char, String, String, bool);

/// A captured frame (rows of `[text, fg, bg, bold]` runs) as cells.
fn want(rows: &Value) -> Vec<Vec<Shown>> {
    let rows = rows.as_array().unwrap();
    rows.iter()
        .map(|row| {
            let runs = row.as_array().unwrap();
            runs.iter()
                .flat_map(|run| {
                    let (fg, bg) = (run[1].as_str().unwrap(), run[2].as_str().unwrap());
                    let bold = run[3].as_bool().unwrap();
                    let text = run[0].as_str().unwrap();
                    text.chars()
                        .map(move |ch| (ch, fg.to_owned(), bg.to_owned(), bold))
                })
                .collect()
        })
        .collect()
}

fn got(g: &Grid) -> Vec<Vec<Shown>> {
    (0..g.height())
        .map(|y| {
            let row = g.row(y).iter();
            row.map(|c| (c.ch, c.fg.to_string(), c.bg.to_string(), c.bold))
                .collect()
        })
        .collect()
}

fn text(row: &[Shown]) -> String {
    row.iter().map(|c| c.0).collect()
}

fn assert_same(name: &str, want: &[Vec<Shown>], got: &[Vec<Shown>]) {
    assert_eq!(want.len(), got.len(), "{name}: rows");
    let mut diffs = vec![];
    for (y, (a, b)) in want.iter().zip(got).enumerate() {
        assert_eq!(a.len(), b.len(), "{name}: row {y} width");
        for (x, (ca, cb)) in a.iter().zip(b).enumerate() {
            if ca != cb {
                diffs.push(format!(
                    "({x},{y}) want {ca:?} got {cb:?}\n  want {:?}\n  got  {:?}",
                    text(a),
                    text(b)
                ));
            }
        }
    }
    assert!(
        diffs.is_empty(),
        "{name}: {} cells differ, first:\n{}",
        diffs.len(),
        diffs[..diffs.len().min(3)].join("\n")
    );
}

#[test]
fn every_frame_of_the_roll_is_the_design_s_cell_for_cell() {
    let file = golden();
    assert_eq!(file["size"], serde_json::json!([W, H]));
    let frames = file["frames"].as_array().unwrap();
    assert_eq!(frames.len() as u64, PERIOD + 1, "frames 0..=PERIOD");
    for fr in frames {
        let f = fr["f"].as_u64().unwrap();
        let g = frame(f);
        assert_eq!((g.width(), g.height()), (W, H));
        assert_same(&format!("frame {f}"), &want(&fr["rows"]), &got(&g));
    }
}

#[test]
fn the_roll_loops_as_the_design_s_does() {
    assert_eq!(golden()["period"].as_u64(), Some(PERIOD));
    for f in 0..PERIOD {
        assert_eq!(frame(f + PERIOD), frame(f), "frame {f}");
        assert_eq!(frame(f + 7 * PERIOD), frame(f), "frame {f}");
    }
    // every frame of one roll is a different picture: the tumbleweed moves on each
    for f in 1..PERIOD {
        assert_ne!(frame(f), frame(f - 1), "frame {f}");
    }
}

#[test]
fn a_frame_shows_for_a_quarter_second() {
    assert_eq!(FRAME_SECONDS, 0.25);
    assert_eq!(index_at(0.0), 0);
    assert_eq!(index_at(0.249), 0);
    assert_eq!(index_at(0.25), 1);
    assert_eq!(index_at(1.0), 4);
    assert_eq!(index_at(10.499), 41);
    assert_eq!(index_at(10.5), 0, "one roll is 10.5 s");
    assert_eq!(index_at(10.75), 1);
    assert_eq!(frame_at(3.6), frame(14));
}

#[test]
fn a_clock_before_the_start_or_broken_shows_the_first_frame() {
    for t in [-1.0, -0.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(index_at(t), 0, "{t}");
    }
    assert_eq!(frame_at(f64::NAN), frame(0));
    // far into a long-open popup the roll still loops
    assert_eq!(index_at(1e9 * FRAME_SECONDS + 0.1), 1_000_000_000 % PERIOD);
}

#[test]
fn the_palette_is_the_design_s() {
    let file = golden();
    let c = &file["C"];
    let named = [
        ("panel", palette::PANEL),
        ("border", palette::BORDER),
        ("text", palette::TEXT),
        ("fg", palette::FG),
        ("dim", palette::DIM),
        ("faint", palette::FAINT),
    ];
    for (k, v) in named {
        assert_eq!(c[k].as_str(), Some(v.to_string().as_str()), "{k}");
    }
}

#[test]
fn a_colour_is_written_as_the_design_writes_it() {
    assert_eq!(Rgb::hex(0x0a0b0c), Rgb(10, 11, 12));
    assert_eq!(Rgb(10, 11, 255).to_string(), "#0a0bff");
}
