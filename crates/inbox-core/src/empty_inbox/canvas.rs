//! A grid of styled cells and the three drawing calls design 2C uses, as the
//! design's `Scr` (herdr-panorama `docs/design/herdr-tui.js`) does them:
//! `put` writes every char of its text, `art` skips blanks, `boxed` draws a
//! filled rounded frame.

use std::fmt;

/// A 24-bit colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// From `0xrrggbb`.
    pub const fn hex(v: u32) -> Self {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
}

impl fmt::Display for Rgb {
    /// `#rrggbb`, lowercase, as the design writes colours.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// One terminal cell. A blank still carries the fg it was written with, as
/// the design's cells do; a renderer may ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
}

/// A `w` × `h` grid of cells, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    w: usize,
    h: usize,
    cells: Vec<Cell>,
}

impl Grid {
    /// Every cell blank in `fg` on `bg` (the design's `new Scr(w, h, bg)`).
    pub fn new(w: usize, h: usize, fg: Rgb, bg: Rgb) -> Self {
        let blank = Cell {
            ch: ' ',
            fg,
            bg,
            bold: false,
        };
        Grid {
            w,
            h,
            cells: vec![blank; w * h],
        }
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    /// The cell at column `x`, row `y`; panics outside the grid.
    pub fn cell(&self, x: usize, y: usize) -> Cell {
        assert!(x < self.w && y < self.h, "({x}, {y}) is outside the grid");
        self.cells[y * self.w + x]
    }

    /// One row of cells.
    pub fn row(&self, y: usize) -> &[Cell] {
        &self.cells[y * self.w..(y + 1) * self.w]
    }

    fn at(&mut self, x: i32, y: i32) -> Option<&mut Cell> {
        let inside = (0..self.w as i32).contains(&x) && (0..self.h as i32).contains(&y);
        inside.then(|| &mut self.cells[y as usize * self.w + x as usize])
    }

    /// Writes `text` from (`x`, `y`) rightwards, blanks included, clipped to
    /// the grid: each cell takes `fg`, `bg` when given, and `bold` (the
    /// design's `put`, which also resets bold when it is not asked for).
    /// Returns the column after the text.
    pub fn put(&mut self, x: i32, y: i32, text: &str, fg: Rgb, bg: Option<Rgb>, bold: bool) -> i32 {
        let mut x = x;
        for ch in text.chars() {
            if let Some(c) = self.at(x, y) {
                c.ch = ch;
                c.fg = fg;
                if let Some(bg) = bg {
                    c.bg = bg;
                }
                c.bold = bold;
            }
            x += 1;
        }
        x
    }

    /// Draws `lines` with its top-left at (`x`, `y`); a blank in them leaves
    /// the cell under it as it was (the design's transparent `art`).
    pub fn art(&mut self, x: i32, y: i32, lines: &[&str], fg: Rgb, bold: bool) {
        for (j, line) in (0..).zip(lines) {
            for (i, ch) in (0..).zip(line.chars()) {
                if ch != ' ' {
                    self.put(x + i, y + j, ch.encode_utf8(&mut [0; 4]), fg, None, bold);
                }
            }
        }
    }

    /// A `w` × `h` rounded frame in `fg` at (`x`, `y`), every cell of the box
    /// blanked on `bg` first (the design's `box(x, y, w, h, col, bg, 'round')`).
    pub fn boxed(&mut self, x: i32, y: i32, w: i32, h: i32, fg: Rgb, bg: Rgb) {
        for j in y..y + h {
            for i in x..x + w {
                if let Some(c) = self.at(i, j) {
                    c.bg = bg;
                    c.ch = ' ';
                }
            }
        }
        let side = "─".repeat((w - 2).max(0) as usize);
        self.put(x, y, &format!("╭{side}╮"), fg, Some(bg), false);
        for j in 1..h - 1 {
            self.put(x, y + j, "│", fg, Some(bg), false);
            self.put(x + w - 1, y + j, "│", fg, Some(bg), false);
        }
        self.put(x, y + h - 1, &format!("╰{side}╯"), fg, Some(bg), false);
    }
}
