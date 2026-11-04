//! Python's `int(s)` for a string (question ids): surrounding whitespace, an optional sign,
//! decimal digits of any script, single underscores between digits.

use unicode_general_category::{GeneralCategory, get_general_category};

use crate::text::py_isspace;

/// The integer, or `None` where `int()` raises `ValueError` (or the value
/// does not fit 64 bits, where Python's SQLite call would overflow).
pub fn parse(s: &str) -> Option<i64> {
    let s = s.trim_matches(py_isspace);
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return None;
    }
    let mut value: i128 = 0;
    for c in digits.chars().filter(|&c| c != '_') {
        let d = digit_value(c)?;
        value = value.checked_mul(10)?.checked_add(i128::from(d))?;
        if value > i128::from(i64::MAX) + 1 {
            return None;
        }
    }
    let value = if negative { -value } else { value };
    i64::try_from(value).ok()
}

/// The value of a Unicode decimal digit (category Nd). Every run of Nd
/// characters is whole decades, each from 0 to 9 (true of every Unicode
/// version Python has used), so it is the digit's distance from its run's
/// start, modulo ten.
fn digit_value(c: char) -> Option<u32> {
    if get_general_category(c) != GeneralCategory::DecimalNumber {
        return None;
    }
    let mut before = 0;
    let mut code = u32::from(c);
    while let Some(p) = code.checked_sub(1).and_then(char::from_u32) {
        if get_general_category(p) != GeneralCategory::DecimalNumber {
            break;
        }
        before += 1;
        code -= 1;
    }
    Some(before % 10)
}

#[cfg(test)]
mod tests;
