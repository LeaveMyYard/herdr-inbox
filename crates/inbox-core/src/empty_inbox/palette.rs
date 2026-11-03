//! Design 2C's colours: the named ones are the design's palette `C`
//! (herdr-panorama `docs/design/herdr-tui.js`), the rest are the scene's own.

use super::Rgb;

/// The popup's fill.
pub const PANEL: Rgb = Rgb::hex(0x171a1f);
/// The popup's frame.
pub const BORDER: Rgb = Rgb::hex(0x80868f);
/// The title and the words.
pub const TEXT: Rgb = Rgb::hex(0xeceef1);
/// What a blank cell is written with (the design's default fg).
pub const FG: Rgb = Rgb::hex(0xc3c7cd);
/// The ground line.
pub const DIM: Rgb = Rgb::hex(0x4d535c);
/// The specks of sand and the tumbleweed's dust.
pub const FAINT: Rgb = Rgb::hex(0x2c313a);
/// The sun.
pub const SUN: Rgb = Rgb::hex(0xc9a861);
/// The dune on the horizon.
pub const DUNE: Rgb = Rgb::hex(0x2a2e35);
/// The cactus.
pub const CACTUS: Rgb = Rgb::hex(0x6c9673);
/// The tumbleweed.
pub const TUMBLEWEED: Rgb = Rgb::hex(0xb49a6a);
