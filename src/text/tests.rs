use super::*;

#[test]
fn squash_splits_on_pythons_whitespace() {
    assert_eq!(squash("  a\tb\n\nc  "), "a b c");
    // U+001C..U+001F, U+0085, NBSP and U+3000 are whitespace to Python's str.split()
    assert_eq!(
        squash("a\u{1c}b\u{1f}c\u{85}d\u{a0}e\u{3000}f"),
        "a b c d e f"
    );
    // U+200B (zero width space) is not
    assert_eq!(squash("a\u{200b}b"), "a\u{200b}b");
    assert_eq!(squash(""), "");
}
