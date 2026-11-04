use super::*;

#[test]
fn strings_quote_as_python_reprs_them() {
    // each checked against Python 3.13's repr()
    assert_eq!(repr("x"), "'x'");
    assert_eq!(repr("it's"), "\"it's\"");
    assert_eq!(repr("a\"b'c"), "'a\"b\\'c'");
    assert_eq!(repr("\t\n\r\\"), "'\\t\\n\\r\\\\'");
    assert_eq!(repr("\u{7}é\u{2028}\u{85}😀"), "'\\x07é\\u2028\\x85😀'");
    assert_eq!(repr("\u{e000}\u{10ffff} "), "'\\ue000\\U0010ffff '");
}
