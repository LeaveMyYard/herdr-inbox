use super::*;

#[test]
fn eight_lowercase_hex_digits_that_differ() {
    let (a, b) = (hex8(), hex8());
    for t in [&a, &b] {
        assert_eq!(t.len(), 8);
        assert!(
            t.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{t}"
        );
    }
    assert_ne!(a, b);
}
