"""Every column the schema declares reaches databases created before it existed.

Final pass 2 of #21: columns added to `CREATE TABLE IF NOT EXISTS` only never reach an
existing database (moneylust ran an earlier head of this branch); remote sync then failed
silently there. BASELINE is each table's column set when the table first shipped: any
other column must be in the migration map."""

import re

from herdr_inbox import store as st
from herdr_inbox.store import Store

BASELINE = {
    "questions": {"id", "created_at", "pane_id", "tab_id", "tab_label", "session_id", "cwd", "asked_by", "question",
                  "options", "context", "norm", "status", "answer", "answered_at", "delivered_at", "delivery_error",
                  "acknowledged_at", "repeat_count"},
    "question_ext": {"id", "header", "multi_select", "answer_json", "group_id", "draft_json", "drafted_at"},
    "sessions": {"session_id", "transcript_path"},
    "meta": {"key", "value"},
    "question_groups": {"group_id", "title"},
    "tab_decor": {"tab_id", "mark"},
    "changes": {"seq", "qid"},
    "applied_requests": {"request_id", "qid", "outcome", "reason", "at"},
    "alerts": {"id", "created_at", "kind", "severity", "title", "message", "pane_id", "tab_id", "tab_label",
               "session_id", "asked_by", "herdr_server", "status", "acked_at", "closed_at", "reported_at"},
    "blocked_panes": {"herdr_server", "pane_id", "since"},
    "remote_alerts": {"machine_id", "db_id", "alert_id", "akey", "status", "data_json", "first_seen_at"},
    "remote_machines": {"machine_id", "label", "target", "db_id", "cursor_seq", "version", "uv", "herdr",
                        "plugin_root", "pulls", "failures", "last_ok_at", "last_error", "last_try_at", "backoff_until"},
    "remote_questions": {"machine_id", "db_id", "remote_id", "qkey", "status", "data_json", "first_seen_at",
                         "notified_at"},
    "remote_outbox": {"request_id", "machine_id", "db_id", "remote_id", "qkey", "action", "detail_json", "created_at",
                      "expires_at", "state", "result", "sent_at", "attempts", "next_try_at"},
}


def declared() -> dict[str, set[str]]:
    out = {}
    for name, body in re.findall(r"CREATE TABLE IF NOT EXISTS (\w+) \((.*?)\n\);", st.SCHEMA, re.S):
        cols = set()
        for line in body.splitlines():
            line = line.strip()
            if line and not line.startswith(("--", "PRIMARY KEY")):
                cols.add(line.split()[0])
        out[name] = cols
    return out


def test_every_declared_column_is_baseline_or_migrated():
    tables = declared()
    assert set(tables) == set(BASELINE), "a new table: add it to BASELINE"
    for table, cols in tables.items():
        added = set(st.TABLE_ADDED_COLUMNS.get(table, {}))
        missing = cols - BASELINE[table] - added
        assert not missing, f"{table}: {sorted(missing)} need an entry in TABLE_ADDED_COLUMNS"


def test_a_database_from_an_earlier_head_gets_the_new_columns(tmp_path):
    """f84d277's schema: remote_questions without issue_notified_at/cleared_at, remote_outbox
    without sending_at."""
    db = tmp_path / "old.db"
    s = Store(db)
    for table, col in (("remote_questions", "issue_notified_at"), ("remote_questions", "cleared_at"),
                       ("remote_outbox", "sending_at")):
        s.db.execute(f"ALTER TABLE {table} DROP COLUMN {col}")
    s.db.execute("PRAGMA user_version = 0")
    s.close()
    s = Store(db)
    for table, cols in declared().items():
        have = {r[1] for r in s.db.execute(f"PRAGMA table_info({table})")}
        assert cols <= have, (table, cols - have)
