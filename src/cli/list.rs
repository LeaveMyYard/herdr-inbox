//! `inbox list [--all]` (Python `cli._command`'s `list`): one line per
//! question, newest first; with `--all` closed ones too, each answer under
//! its question.

use inbox_core::popup_view::{age, first_line};

use super::Io;
use crate::store::Store;

/// Prints the questions; always 0.
pub fn list(store: &Store, all: bool, io: &mut Io<'_>) -> rusqlite::Result<i32> {
    for q in store.list(all)? {
        let nonempty = |s: &Option<String>| s.clone().filter(|s| !s.is_empty());
        let place = nonempty(&q.tab_label)
            .or_else(|| nonempty(&q.pane_id))
            .unwrap_or_else(|| "-".to_string());
        let by = nonempty(&q.asked_by).map_or_else(String::new, |b| format!(" [{b}]"));
        let rep = if q.repeat_count != 0 {
            format!(" x{}", q.repeat_count + 1)
        } else {
            String::new()
        };
        let _ = writeln!(
            io.out,
            "{:>5} {:<9} {:>4} {place}{by}{rep}: {}",
            format!("Q{}", q.id),
            q.status,
            age(&q.created_at, io.now),
            first_line(&q.question, 100)
        );
        if all && let Some(answer) = nonempty(&q.answer) {
            let _ = writeln!(io.out, "      -> {answer}");
        }
    }
    Ok(0)
}
