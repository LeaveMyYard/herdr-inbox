use super::{LONG, Text};

#[test]
fn a_cut_stops_at_a_character_boundary_within_the_limit() {
    let s = format!("{}é", "a".repeat(LONG - 1));
    let mut cut = false;
    let out = Text::Cut.apply(&s, LONG, &mut cut);
    assert!(cut);
    assert_eq!(out.len(), LONG - 1);
    assert_eq!(out, "a".repeat(LONG - 1));
}

#[test]
fn text_at_the_limit_is_not_cut() {
    let s = "a".repeat(LONG);
    let mut cut = false;
    assert_eq!(Text::Cut.apply(&s, LONG, &mut cut), s);
    assert!(!cut);
}

#[test]
fn whole_text_is_never_cut() {
    let s = "a".repeat(LONG + 1);
    let mut cut = false;
    assert_eq!(Text::Whole.apply(&s, LONG, &mut cut), s);
    assert!(!cut);
}
