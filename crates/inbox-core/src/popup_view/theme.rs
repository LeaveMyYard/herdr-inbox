//! The popup's colours: Claude Code's theme roles for its question prompt
//! (Python `tui.CC_COLORS`) on the base colours of Textual's `textual-dark`
//! theme, which the Python popup runs on (Textual 8.2.8; the app-screen
//! goldens hold them).

use crate::empty_inbox::Rgb;

/// Claude Code's roles (read from the 2.1.299 bundle): the pointer and the
/// focused option are `suggestion`, a chosen answer `success`, and
/// `warning` only warns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CcColors {
    /// `inbox-suggestion`.
    pub suggestion: Rgb,
    /// `inbox-success`.
    pub success: Rgb,
    /// `inbox-warning`.
    pub warning: Rgb,
}

/// Python `CC_COLORS["dark"]` (its `light` values come with a light theme;
/// the popup runs only on `textual-dark`).
pub const CC_DARK: CcColors = CcColors {
    suggestion: Rgb::hex(0xb1b9f9),
    success: Rgb::hex(0x4eba65),
    warning: Rgb::hex(0xffc107),
};

/// Every colour the popup paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Claude Code's roles.
    pub cc: CcColors,
    /// `$background`: the screen.
    pub background: Rgb,
    /// `$foreground`: text.
    pub foreground: Rgb,
    /// A widget coloured `$text-muted` (meta lines, hints).
    pub muted: Rgb,
    /// `[$text-muted]` inside markup (the option numbers): Textual resolves
    /// the auto colour without its alpha.
    pub number: Rgb,
    /// `$panel`: the dismiss confirmation's fill.
    pub panel: Rgb,
    /// A focused `Input`'s fill (`$surface` tinted 5% `$foreground`).
    pub input: Rgb,
    /// `$border`: a focused `Input`'s frame.
    pub border: Rgb,
    /// `$input-selection-background` over the `Input`: its selected text.
    pub selection: Rgb,
    /// `$text-disabled` over the `Input`: an empty one's placeholder.
    pub disabled: Rgb,
    /// `$scrollbar`: the thumb.
    pub thumb: Rgb,
    /// `$scrollbar-background`: the track.
    pub track: Rgb,
}

impl Theme {
    /// The popup as it runs (Textual's default `textual-dark`).
    pub const DARK: Theme = Theme {
        cc: CC_DARK,
        background: Rgb::hex(0x121212),
        foreground: Rgb::hex(0xe0e0e0),
        muted: Rgb::hex(0xa0a0a0),
        number: Rgb::hex(0xffffff),
        panel: Rgb::hex(0x242f38),
        input: Rgb::hex(0x272727),
        border: Rgb::hex(0x0178d4),
        selection: Rgb::hex(0x2d4e74),
        disabled: Rgb::hex(0x797979),
        thumb: Rgb::hex(0x003054),
        track: Rgb::hex(0x000000),
    };
}

impl Default for Theme {
    fn default() -> Self {
        Theme::DARK
    }
}

/// Textual's `Color.blend`: `factor` of the way from `a` to `b`, truncated.
pub(crate) fn blend(a: Rgb, b: Rgb, factor: f64) -> Rgb {
    let mix = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * factor) as u8;
    Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

/// Textual's `dim` (`DIM_FACTOR` 0.66): a third of the way to the background.
pub(crate) fn dim(fg: Rgb, bg: Rgb) -> Rgb {
    blend(fg, bg, 1.0 - 0.66)
}
