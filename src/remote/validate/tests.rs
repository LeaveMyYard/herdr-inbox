use super::*;

#[test]
fn the_size_cap_is_two_million_characters() {
    // Python: len(raw) > MAX_EXPORT_BYTES (characters of the text)
    assert_eq!(MAX_EXPORT_BYTES, 2_000_000);
    let over = "x".repeat(2_000_001); // literal: the cap itself is what is pinned
    assert_eq!(
        validate_export(&over),
        Err(ExportError::Bad("export too large".into()))
    );
    let at = "x".repeat(2_000_000);
    assert!(
        !matches!(validate_export(&at), Err(ExportError::Bad(m)) if m == "export too large"),
        "exactly the cap is read"
    );
    let wide = "é".repeat(2_000_000); // 4 MB of UTF-8, 2,000,000 characters
    assert!(
        !matches!(validate_export(&wide), Err(ExportError::Bad(m)) if m == "export too large"),
        "characters, not bytes"
    );
}
