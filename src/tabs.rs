//! The inbox's tab marks (Python `tabs`): the part the store's alerts and the
//! MCP tools' caller need.

use unicode_general_category::{GeneralCategory, get_general_category};

use crate::store::Store;

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

/// Python `tabs.marks_for`: the default mark, plus every mark the inbox
/// applied to `tab` (`tab_decor`), the empty one left out.
pub fn marks_for(store: &Store, tab: Option<&str>) -> rusqlite::Result<Vec<String>> {
    let mut marks = vec![DEFAULT_PREFIX.to_owned()];
    if let Some(tab) = tab.filter(|t| !t.is_empty()) {
        let mut stmt = store
            .db()
            .prepare("SELECT mark FROM tab_decor WHERE tab_id = ?")?;
        for mark in stmt.query_map([tab], |r| r.get::<_, Option<String>>(0))? {
            if let Some(m) = mark?
                && !m.is_empty()
                && !marks.contains(&m)
            {
                marks.push(m);
            }
        }
    }
    Ok(marks)
}

#[cfg(test)]
mod tests;
