//! The desert, drawn as design 2C's `inboxC(f)` draws it: the popup frame,
//! the sun, a dune, a cactus, the ground and its specks of sand, the words,
//! and the tumbleweed rolling left to right on frame `f`.

use super::palette::{BORDER, CACTUS, DIM, DUNE, FAINT, FG, PANEL, SUN, TEXT, TUMBLEWEED};
use super::{Grid, H, PERIOD, W};

/// The ground's row.
const GROUND: i32 = 12;
/// The words under the scene.
const WORDS: &str = "No open questions";

/// The tumbleweed's three turns, one per frame in a cycle.
const TURNS: [[&str; 3]; 3] = [
    [" ⡠⠔⠒⢄ ", "⢸⠡⡈⢂⠜⡆", " ⠑⠢⠔⠊ "],
    [" ⡔⠒⠢⢄ ", "⢸⠑⡄⡠⢁⡇", " ⠣⠤⠔⠉ "],
    [" ⢀⠔⠒⢤ ", "⡜⢂⠌⡢⠈⡇", " ⠈⠢⠤⠚ "],
];

/// Frame `f` of the scene; it repeats every [`PERIOD`] frames.
pub(super) fn draw(f: u64) -> Grid {
    let mut s = Grid::new(W, H, FG, PANEL);
    s.boxed(0, 0, W as i32, H as i32, BORDER, PANEL);
    s.put(2, 0, " inbox ", TEXT, Some(PANEL), true);
    s.art(22, 3, &[" ▄▄ ", "▀██▀"], SUN, false);
    s.art(47, GROUND - 2, &["   ▁▄▄▄▄▄▁", " ▁▟███████▙▁"], DUNE, false);
    let cactus = ["  ╷  ", "╷ │  ", "╰─┤ ╷", "  ├─╯", "  │  "];
    s.art(40, GROUND - 5, &cactus, CACTUS, true);
    let ground = "▁".repeat(W - 2);
    s.put(1, GROUND, &ground, DIM, Some(PANEL), false);
    let sand = " ·    ˙       ·         ˙     ·      ·  ˙        ·   ";
    s.put(3, GROUND + 1, sand, FAINT, Some(PANEL), false);
    tumbleweed(&mut s, (f % PERIOD) as i32);
    let x = (W - WORDS.chars().count()) as i32 / 2;
    s.put(x, 15, WORDS, TEXT, Some(PANEL), false);
    s
}

/// The tumbleweed `p` frames into its roll: one column a frame, a hop up on
/// every fourth, and a trail of dust once it is under way.
fn tumbleweed(s: &mut Grid, p: i32) {
    let x = 2 + p;
    let hop = i32::from(p % 4 == 1);
    s.art(
        x,
        GROUND - 3 - hop,
        &TURNS[(p % 3) as usize],
        TUMBLEWEED,
        true,
    );
    if p > 2 {
        s.put(x - 3, GROUND - 1, "˙ ·", FAINT, None, false);
    }
}
