//! The empty inbox, design 2C (operator pick, Q179): a tumbleweed rolling
//! through the desert over "No open questions", in a 64 × 20 popup.
//!
//! A pure frame generator: a frame index, or the seconds since the empty
//! inbox came up, in; a [`Grid`] of styled cells out. It reproduces the
//! design's own cell-grid renderer, `inboxC(f)` in herdr-panorama
//! `docs/design/herdr-tui.js`, cell for cell (char, fg, bg, bold), checked
//! against frames captured from it (`tests/fixtures/design_empty_inbox.json`,
//! `tools/capture_empty_inbox.mjs`). The popup renderer draws the grid; this
//! module knows nothing of terminals.
//!
//! - [`Grid`] of [`Cell`]s: what a frame is (`canvas`, with the drawing calls).
//! - [`palette`]: the scene's colours.

mod canvas;
pub mod palette;
mod scene;

pub use canvas::{Cell, Grid, Rgb};

/// The popup's width in cells.
pub const W: usize = 64;
/// The popup's height in cells.
pub const H: usize = 20;
/// Frames in one roll; frame `f + PERIOD` is frame `f` (the design's
/// `f % (span + 8)`, span 34).
pub const PERIOD: u64 = 42;
/// Seconds a frame shows, so one roll takes 10.5 s. The design takes only a
/// frame index; its project's prototype pages advance it every 250 ms
/// (`setInterval(…, 250)` in herdr-panorama `docs/design/Swarm View
/// 2a.dc.html`; the 2C page itself could not be fetched), and herdr-panorama's
/// port runs its design frames at 250 ms too (`view/anim`).
pub const FRAME_SECONDS: f64 = 0.25;

/// Frame `f` of the scene.
pub fn frame(f: u64) -> Grid {
    scene::draw(f)
}

/// The frame showing `elapsed` seconds after the empty inbox came up;
/// a negative or non-finite time shows the first frame.
pub fn frame_at(elapsed: f64) -> Grid {
    frame(index_at(elapsed))
}

/// The frame index at `elapsed` seconds: whole frames passed, as a
/// renderer ticking every [`FRAME_SECONDS`] counts them.
pub fn index_at(elapsed: f64) -> u64 {
    if elapsed.is_finite() && elapsed > 0.0 {
        (elapsed / FRAME_SECONDS) as u64 % PERIOD
    } else {
        0
    }
}
