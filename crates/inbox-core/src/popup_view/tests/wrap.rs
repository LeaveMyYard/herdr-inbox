//! Textual's wrap: what `compute_wrap_offsets` and `_wrap_and_format` give.

use crate::popup_view::text::{Line, Ln, Look};
use crate::popup_view::wrap::{offsets, wrap};

fn plain(line: &Line) -> String {
    line.iter().map(|s| s.text.as_str()).collect()
}

fn lines(text: &str, width: usize) -> Vec<String> {
    wrap(&Ln::new().plain(text).done(), width)
        .iter()
        .map(plain)
        .collect()
}

// The expectations are Textual 8.2.8's `Content(text).wrap(width)`.

#[test]
fn a_word_that_does_not_fit_goes_to_the_next_line_its_trailing_space_aside() {
    assert_eq!(lines("Esc to cancel", 6), ["Esc to", "cancel"]);
    assert_eq!(lines("ab cd", 5), ["ab cd"]);
    assert_eq!(lines("ab cd ", 5), ["ab cd"]);
    assert_eq!(lines("ab cd ef", 5), ["ab cd", "ef"]);
}

#[test]
fn every_line_but_the_last_loses_its_trailing_whitespace_and_the_last_is_cropped() {
    assert_eq!(lines("one two  three", 8), ["one two", "three"]);
    assert_eq!(lines("  lead", 3), ["  l", "ead"]);
    assert_eq!(lines("x\u{a0}y z", 3), ["x\u{a0}y", "z"]);
}

#[test]
fn a_word_wider_than_the_line_folds() {
    assert_eq!(lines("abcdefgh", 3), ["abc", "def", "gh"]);
    assert_eq!(lines("hi abcdefgh", 4), ["hi", "abcd", "efgh"]);
    assert_eq!(offsets(&"abcdefgh".chars().collect::<Vec<_>>(), 3), [3, 6]);
}

#[test]
fn a_wide_char_never_straddles_the_edge() {
    assert_eq!(lines("a⏳b", 2), ["a", "⏳", "b"]);
}

#[test]
fn spans_keep_their_looks_across_the_breaks() {
    let bold = Look::default().with_bold();
    let line = Ln::new().plain("ab ").on(bold, "cd ef").done();
    let rows = wrap(&line, 4);
    assert_eq!(
        rows.iter().map(plain).collect::<Vec<_>>(),
        ["ab", "cd", "ef"]
    );
    assert_eq!(rows[1][0].look, bold);
    assert_eq!(rows[2][0].look, bold);
}

#[test]
fn tabs_expand_to_the_next_stop() {
    assert_eq!(lines("a\tb", 20), ["a       b"]);
}

#[test]
fn a_zero_width_line_breaks_nowhere() {
    assert!(offsets(&['a', 'b'], 0).is_empty());
}
