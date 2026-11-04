//! The questions and alerts `tools/capture_goldens.py` files, as the popup
//! reads them: the three callers, `tests/make_screenshots.py`'s demo data,
//! and the held clock (`hold`: Q<n> was asked 7 × (10 − n) minutes, A<n>
//! raised 3 × (10 − n) hours before 2026-10-17T12:00:00Z).

use inbox_core::popup::{Alert, QOption, Question, Snapshot};

/// `HELD`, Unix seconds.
pub const HELD: i64 = 1_792_238_400;

/// A caller: pane and tab label.
#[derive(Clone, Copy)]
pub struct Caller(pub &'static str, pub &'static str);

pub const BILLING: Caller = Caller("w1:p1", "Billing API");
pub const SEARCH: Caller = Caller("w1:p2", "Search refactor");
pub const DOCS: Caller = Caller("w1:p3", "Docs site");

fn iso(unix: i64) -> String {
    let days = unix.div_euclid(86400);
    let secs = unix.rem_euclid(86400);
    let (y, m, d) = civil(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// Howard Hinnant's `civil_from_days`.
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Labels only.
pub fn opts(labels: &[&str]) -> Vec<QOption> {
    labels.iter().map(|l| QOption::label(*l)).collect()
}

/// Python's `DB` options.
pub fn db() -> Vec<QOption> {
    let mut o = opts(&["Postgres", "SQLite", "DuckDB"]);
    o[0].description = Some("keeps the current ORM and migrations".into());
    o[1].description = Some("single file, no server".into());
    o
}

/// An open question `id` of group `group`, asked by `who`.
pub fn question(
    id: i64,
    group: i64,
    who: Caller,
    text: &str,
    options: Vec<QOption>,
    header: &str,
) -> Question {
    Question {
        id,
        group,
        status: "open".into(),
        pane_id: Some(who.0.into()),
        tab_label: Some(who.1.into()),
        options,
        header: Some(header.into()).filter(|h: &String| !h.is_empty()),
        question: text.into(),
        created_at: iso(HELD - 7 * 60 * (10 - id)),
        ..Question::default()
    }
}

/// An open alert `id` of `kind`.
pub fn alert(id: i64, who: Caller, title: &str, message: &str, severity: &str, by: &str) -> Alert {
    Alert {
        id,
        kind: "agent".into(),
        status: "open".into(),
        pane_id: Some(who.0.into()),
        severity: severity.into(),
        title: title.into(),
        message: Some(message.into()),
        tab_label: Some(who.1.into()),
        asked_by: Some(by.into()),
        created_at: iso(HELD - 3 * 3600 * (10 - id)),
    }
}

/// A snapshot of these questions and alerts (the open ones listed, in id
/// order; `failed` the undelivered answers).
pub fn snapshot(questions: Vec<Question>, failed: Vec<i64>, alerts: Vec<Alert>) -> Snapshot {
    let mut s = Snapshot::default();
    s.open = questions
        .iter()
        .filter(|q| q.status == "open")
        .map(|q| q.id)
        .collect();
    s.open_alerts = alerts
        .iter()
        .filter(|a| a.status == "open")
        .map(|a| a.id)
        .collect();
    s.questions = questions;
    s.failed = failed;
    s.alerts = alerts;
    s
}

/// Python `populate`: Q1 (search, alone), Q2 + Q3 (billing, one titled
/// call with context), Q4 (docs).
pub fn populate() -> Vec<Question> {
    let mut q1 = question(
        1,
        1,
        SEARCH,
        "Merge PR #42 with a merge commit or squash?",
        opts(&["merge commit", "squash"]),
        "Merge mode",
    );
    q1.asked_by = Some("reviewer".into());
    let call = |id, text: &str, options, header: &str| {
        let mut q = question(id, 2, BILLING, text, options, header);
        q.context = Some("Invoice rows ~1M/day".into());
        q.asked_by = Some("planner".into());
        q.group_title = Some("Invoice storage and caching".into());
        q
    };
    let q2 = call(
        2,
        "Which database should the invoice store use?",
        db(),
        "DB choice",
    );
    let mut q3 = call(
        3,
        "Which caches should the API warm on deploy?",
        opts(&["redis", "in-process", "CDN"]),
        "Cache",
    );
    q3.multi_select = true;
    let q4 = question(
        4,
        4,
        DOCS,
        "Publish the v2 docs now or after the release?",
        opts(&["now", "after"]),
        "Rollout",
    );
    vec![q1, q2, q3, q4]
}
