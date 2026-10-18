use super::*;

#[test]
fn the_first_line_is_pythons() {
    // Python: service_first_line(t) for each
    assert_eq!(first_line("  a\nb  "), "a");
    assert_eq!(first_line("x\u{2028}y"), "x");
    assert_eq!(first_line("x\u{1c}y"), "x");
    assert_eq!(first_line("\r\nz"), "z");
    assert_eq!(first_line("   "), "");
    assert_eq!(first_line(&"é".repeat(300)).chars().count(), 200);
}
