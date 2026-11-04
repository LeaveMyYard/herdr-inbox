//! One tab's re-read and rename (Python `tabs.apply`). Called under
//! [`super::lock::marker_lock`] with the count read under it.

use rusqlite::{Connection, params};

use super::base_label;
use super::label::{desired_label, marks_for};
use crate::surface::Surface;

/// Re-reads `tab`'s label and renames it to show `count` if it differs.
///
/// The tab's mark is recorded (`tab_decor`) before a decorating rename, so
/// only tabs the inbox decorated are ever stripped of a custom mark. A tab
/// left (or found) at its base is no longer the inbox's, unless the rename
/// back to it failed. A tab herdr does not report is left alone.
pub fn apply(
    db: &Connection,
    herdr: &mut dyn Surface,
    tab: &str,
    count: i64,
    mark: &str,
    prefix_unnamed: impl FnOnce() -> bool,
) -> rusqlite::Result<()> {
    let Some(current) = herdr.tab_label(tab) else {
        return Ok(());
    };
    let marks = marks_for(db, Some(tab))?;
    let marks: Vec<&str> = marks.iter().map(String::as_str).collect();
    let base = base_label(&current, &marks);
    let want = desired_label(&base, count, mark, prefix_unnamed);
    if want != current {
        if want != base {
            db.execute(
                "INSERT OR IGNORE INTO tab_decor (tab_id, mark) VALUES (?, ?)",
                params![tab, mark],
            )?;
        }
        if herdr.rename_tab(tab, &want).is_err() {
            return Ok(());
        }
    }
    if want == base {
        db.execute("DELETE FROM tab_decor WHERE tab_id = ?", [tab])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
