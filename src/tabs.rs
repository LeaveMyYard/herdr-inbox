//! The inbox's tab marks (Python `tabs`): the part the store's alerts need.

use unicode_general_category::{GeneralCategory, get_general_category};

/// The default mark in front of a tab's count (Python `tabs.DEFAULT_PREFIX`).
pub const DEFAULT_PREFIX: &str = "❓";

/// The operator's label: every leading `<mark><digits> ` of ours stripped
/// (Python `tabs.base_label`; `\d` is any Unicode decimal digit, as in `re`).
pub fn base_label(label: &str, marks: &[&str]) -> String {
    // an empty mark strips bare `<digits> ` too, as in Python (its callers leave "" out: marks_for)
    let mut marks: Vec<&str> = marks.to_vec();
    marks.sort_unstable();
    let mut label = label;
    'strip: loop {
        for mark in &marks {
            if let Some(rest) = label.strip_prefix(mark) {
                let digits = rest
                    .chars()
                    .take_while(|&c| get_general_category(c) == GeneralCategory::DecimalNumber);
                let len: usize = digits.map(char::len_utf8).sum();
                if len > 0 && rest[len..].starts_with(' ') {
                    label = &rest[len + 1..];
                    continue 'strip;
                }
            }
        }
        return label.to_string();
    }
}

#[cfg(test)]
mod tests;
