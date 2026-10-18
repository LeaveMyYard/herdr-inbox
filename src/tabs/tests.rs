use super::*;

#[test]
fn every_leading_mark_of_ours_goes() {
    // Python: tabs.base_label("❓2 ❓١ web ❓3 x", {"❓"}) == "web ❓3 x" (\d is Unicode)
    assert_eq!(
        base_label("❓2 ❓\u{661} web ❓3 x", &[DEFAULT_PREFIX]),
        "web ❓3 x"
    );
    assert_eq!(
        base_label("❓ web", &[DEFAULT_PREFIX]),
        "❓ web",
        "no digits: not ours"
    );
    assert_eq!(
        base_label("❓2web", &[DEFAULT_PREFIX]),
        "❓2web",
        "no space: not ours"
    );
    assert_eq!(base_label("!3 ❓1 a", &["❓", "!"]), "a");
    assert_eq!(base_label("❓1 a", &[]), "❓1 a");
    // Python: base_label("12 x", {""}) == "x", base_label("❓1 12 x", {"", "❓"}) == "x", base_label("❓2 ", {"❓"}) == ""
    assert_eq!(base_label("12 x", &[""]), "x");
    assert_eq!(base_label("❓1 12 x", &["", "❓"]), "x");
    assert_eq!(base_label("❓2 ", &[DEFAULT_PREFIX]), "");
}
