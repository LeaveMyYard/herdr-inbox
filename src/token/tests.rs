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

#[test]
fn a_request_id_is_32_lowercase_hex_digits_that_differ() {
    let (a, b) = (request_id(), request_id());
    assert_eq!(a.len(), 32);
    assert!(
        a.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "{a}"
    );
    assert_ne!(a, b);
}
