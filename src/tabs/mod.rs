//! The inbox's tab marks (Python `tabs`): the open-question count in front
//! of a tab's label, `❓2 Billing API`.
//!
//! herdr has no per-tab tokens, so the count goes into the label itself. The
//! operator's own label (the *base*) is never stored: it is recovered by
//! stripping our marks from the label herdr reports now ([`base_label`]), so
//! a rename by the operator is adopted and a crash leaves at worst a stale
//! prefix that the next `inbox sync-tokens` removes.
//!
//! - [`settings`]: `tab_prefix` and `prefix_unnamed_tabs` from the plugin's
//!   `config.toml`, read on every use.
//! - [`label`]: the marks a tab may be stripped of, the label it should
//!   have, the counts per tab.
//! - [`lock`]: the cross-process lock every count, re-read and rename runs
//!   under (`<db>.markers.lock`, never the database's own lock).
//! - [`apply`]: one tab's re-read and rename.

pub mod apply;
pub mod label;
pub mod lock;
pub mod settings;

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
