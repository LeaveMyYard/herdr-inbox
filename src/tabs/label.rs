//! What a tab's label should be (Python `tabs.marks_for`, `desired_label`,
//! `tab_counts`).

use std::collections::BTreeMap;

use rusqlite::Connection;
use unicode_general_category::{GeneralCategory, get_general_category};

use super::DEFAULT_PREFIX;

/// The marks that may be stripped from `tab`'s label: the default, plus any
/// the inbox applied to it (`tab_decor`), the empty mark left out. Sorted
/// (Python's is a set; [`super::base_label`] sorts anyway).
pub fn marks_for(db: &Connection, tab: Option<&str>) -> rusqlite::Result<Vec<String>> {
    let mut marks = vec![DEFAULT_PREFIX.to_string()];
    if let Some(tab) = tab.filter(|t| !t.is_empty()) {
        let mut stmt = db.prepare("SELECT mark FROM tab_decor WHERE tab_id = ?")?;
        for mark in stmt.query_map([tab], |r| r.get::<_, String>(0))? {
            marks.push(mark?);
        }
    }
    marks.retain(|m| !m.is_empty());
    marks.sort_unstable();
    marks.dedup();
    Ok(marks)
}

/// The label a tab whose base is `base` shows for `count` open questions:
/// `{mark}{count} {base}`, or `base` when there are none, the prefix is off,
/// or the tab is unnamed (its label is its position) and `prefix_unnamed()`
/// is off. The setting is read only for an unnamed tab with a count, as
/// Python reads it.
pub fn desired_label(
    base: &str,
    count: i64,
    mark: &str,
    prefix_unnamed: impl FnOnce() -> bool,
) -> String {
    if count <= 0 || mark.is_empty() || (py_isdigit(base) && !prefix_unnamed()) {
        return base.to_string();
    }
    format!("{mark}{count} {base}")
}

/// The open counts per tab, from the counts per pane and each pane's tab
/// (panes without a tab, or unknown to herdr, count nowhere).
pub fn tab_counts(
    pane_counts: &BTreeMap<String, i64>,
    pane_tabs: &[(String, Option<String>)],
) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for (pane, n) in pane_counts {
        // the last listing of a pane wins, as in Python's dict
        let tab = pane_tabs
            .iter()
            .rev()
            .find(|(p, _)| p == pane)
            .and_then(|(_, t)| t.as_deref())
            .filter(|t| !t.is_empty());
        if let Some(tab) = tab {
            *out.entry(tab.to_string()).or_insert(0) += n;
        }
    }
    out
}

/// Python's `str.isdigit`: non-empty, every character a decimal digit or a
/// digit of another kind (superscripts, circled digits, ...).
pub fn py_isdigit(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            get_general_category(c) == GeneralCategory::DecimalNumber
                || OTHER_DIGITS
                    .iter()
                    .any(|&(lo, hi)| (lo..=hi).contains(&u32::from(c)))
        })
}

/// The characters `str.isdigit` accepts outside category Nd (Python 3.13:
/// `[c for c in chars if c.isdigit() and category(c) != "Nd"]`, as ranges).
const OTHER_DIGITS: [(u32, u32); 20] = [
    (0xB2, 0xB3),
    (0xB9, 0xB9),
    (0x1369, 0x1371),
    (0x19DA, 0x19DA),
    (0x2070, 0x2070),
    (0x2074, 0x2079),
    (0x2080, 0x2089),
    (0x2460, 0x2468),
    (0x2474, 0x247C),
    (0x2488, 0x2490),
    (0x24EA, 0x24EA),
    (0x24F5, 0x24FD),
    (0x24FF, 0x24FF),
    (0x2776, 0x277E),
    (0x2780, 0x2788),
    (0x278A, 0x2792),
    (0x10A40, 0x10A43),
    (0x10E60, 0x10E68),
    (0x11052, 0x1105A),
    (0x1F100, 0x1F10A),
];

#[cfg(test)]
mod tests;
