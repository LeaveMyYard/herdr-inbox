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
    s = Store(db, migrate=True)
    for table, col in (("remote_questions", "issue_notified_at"), ("remote_questions", "cleared_at"),
                       ("remote_outbox", "sending_at")):
        s.db.execute(f"ALTER TABLE {table} DROP COLUMN {col}")
    s.db.execute("PRAGMA user_version = 0")
    s.close()
    s = Store(db, migrate=True)
    for table, cols in declared().items():
        have = {r[1] for r in s.db.execute(f"PRAGMA table_info({table})")}
        assert cols <= have, (table, cols - have)


# -- P0b: the integer revision; only the worker and the popup migrate, forward only ------------

import sqlite3  # noqa: E402
import threading  # noqa: E402
import time  # noqa: E402

import pytest  # noqa: E402

from herdr_inbox.store import SchemaNotReady, revision_stamp, stamped_revision  # noqa: E402

from .compat_prebridge import (  # noqa: E402
    CURRENT_PRE_BRIDGE, LEGACY_STAMPS, columns, make_older_head, pre_bridge_open, schema_statements, set_stamp,
    stamp_of, traced)

UNKNOWN_STAMPS = [*LEGACY_STAMPS.values(), 0, 1, 2, 0x7FFFFFFF]  # 1, 2: a crc32 can be small too


def schema_writes(statements: list[str]) -> list[str]:
    """Schema work, the stamp, or the write lock (no data writes happen in these tests)."""
    return schema_statements(statements) + [s for s in statements if s.lstrip().upper().startswith("BEGIN IMMEDIATE")]


@pytest.fixture
def migrated(tmp_path):
    path = tmp_path / "inbox.db"
    Store(path, migrate=True).close()
    return path


def test_revisions_live_in_the_sign_bit_so_no_crc32_is_ever_a_revision():
    assert revision_stamp() < 0 and stamped_revision(revision_stamp()) == st.MIGRATION_REVISION
    assert stamped_revision(revision_stamp(st.MIGRATION_REVISION + 1)) == st.MIGRATION_REVISION + 1
    for value in UNKNOWN_STAMPS:  # every crc32 is 0..2**31-1: masked with 0x7FFFFFFF since 3c0ddfb
        assert stamped_revision(value) is None
    with pytest.raises(ValueError):
        revision_stamp(0)


def test_the_stamp_no_longer_depends_on_the_migration_source(migrated):
    """The crc32 over Store._migrate's source changed with every comment edit; a Rust port
    could never share it. The stamp is the integer revision now."""
    assert stamp_of(migrated) == revision_stamp(st.MIGRATION_REVISION)
    assert not hasattr(st, "SCHEMA_STAMP") and not hasattr(st, "_schema_stamp")


@pytest.mark.parametrize("legacy", UNKNOWN_STAMPS)
def test_any_stamp_that_is_not_a_known_revision_migrates_forward(migrated, legacy):
    """A crc32 left by a pre-bridge process (however large: 2042562191 > any revision as a
    plain int), 0 from a fresh or pre-stamp database: the worker migrates forward."""
    with Store(migrated, maintenance=False) as s:
        s.db.execute("INSERT INTO questions (created_at, question, norm) VALUES ('t', 'Kept?', 'kept')")
    make_older_head(migrated, legacy)
    with Store(migrated, migrate=True, maintenance=False) as s:
        assert "sending_at" in columns(migrated, "remote_outbox")
        assert [r[0] for r in s.db.execute("SELECT question FROM questions")] == ["Kept?"]  # no data lost
    assert stamp_of(migrated) == revision_stamp()


def test_a_higher_revision_is_never_lowered_nor_migrated_again(migrated, monkeypatch):
    """A newer release (the Rust worker) stamped a higher revision: nothing here touches it."""
    make_older_head(migrated, revision_stamp(st.MIGRATION_REVISION + 1))  # missing column: proves no migration ran
    statements = traced(monkeypatch)
    for migrate in (True, False):
        Store(migrated, migrate=migrate, maintenance=False).close()
    assert stamp_of(migrated) == revision_stamp(st.MIGRATION_REVISION + 1)
    assert "sending_at" not in columns(migrated, "remote_outbox")
    assert schema_writes(statements) == []


def test_the_migration_rechecks_under_the_lock_and_never_lowers_a_higher_revision(migrated, monkeypatch):
    """Another worker (a newer one) stamped while this one waited for BEGIN IMMEDIATE."""
    set_stamp(migrated, CURRENT_PRE_BRIDGE)
    s = Store(migrated, migrate=True, maintenance=False)
    set_stamp(migrated, revision_stamp(st.MIGRATION_REVISION + 1))
    s._migrate()
    s.close()
    assert stamp_of(migrated) == revision_stamp(st.MIGRATION_REVISION + 1)


def test_a_non_migrating_open_on_a_stale_stamp_uses_the_db_without_any_write(migrated, monkeypatch):
    """Hooks, MCP tools, the CLI on a database a pre-bridge process re-stamped: the schema is
    all there, so they work, never take the write lock for it and never touch the stamp."""
    set_stamp(migrated, CURRENT_PRE_BRIDGE)
    holder = sqlite3.connect(migrated, isolation_level=None)
    holder.execute("BEGIN IMMEDIATE")  # the write lock is busy: a migrating open would wait 0.2 s and fail
    statements = traced(monkeypatch)  # (after the holder: the patch is process-wide)
    try:
        started = time.monotonic()
        with Store(migrated, timeout=0.2, maintenance=False) as s:
            assert s.list() == []
        assert time.monotonic() - started < 0.2
    finally:
        holder.execute("ROLLBACK")
        holder.close()
    assert stamp_of(migrated) == CURRENT_PRE_BRIDGE
    assert schema_writes(statements) == []


def test_a_non_migrating_open_on_an_older_schema_raises_and_writes_nothing(migrated, tmp_path, monkeypatch):
    make_older_head(migrated)
    statements = traced(monkeypatch)
    with pytest.raises(SchemaNotReady, match="remote_outbox.sending_at"):
        Store(migrated, maintenance=False)
    with pytest.raises(SchemaNotReady):
        Store(migrated)  # with the reconcile pass too: it never runs
    with pytest.raises(SchemaNotReady):
        Store(tmp_path / "fresh.db")  # a fresh install before the worker started
    assert schema_writes(statements) == []
    assert stamp_of(migrated) == CURRENT_PRE_BRIDGE
    assert "sending_at" not in columns(migrated, "remote_outbox")


def test_every_schema_object_is_checked(migrated):
    """A missing view, index or trigger is as unready as a missing column."""
    assert st.schema_missing(sqlite3.connect(migrated)) == []
    for kind, name in (("VIEW", st.VIEW), ("INDEX", "questions_status"), ("TRIGGER", "changes_x_upd")):
        db = sqlite3.connect(migrated, isolation_level=None)
        db.execute(f"DROP {kind} {name}")
        assert st.schema_missing(db) == [name]
        db.close()
        Store(migrated, migrate=True).close()  # unstamped below: put it back
        set_stamp(migrated, 0)
        Store(migrated, migrate=True).close()


def test_pre_bridge_and_bridge_processes_interleave_without_a_fight_or_data_loss(migrated):
    """A 0.10.5 MCP server keeps re-stamping its crc32 until its session restarts: each of its
    opens costs the bridge worker one idempotent pass, and the bridge's hooks never write."""
    for i in range(3):
        pre_bridge_open(migrated)  # the old server re-stamps (and finds its schema all there)
        assert stamp_of(migrated) == CURRENT_PRE_BRIDGE
        with Store(migrated, maintenance=False) as hook_side:  # a bridge hook / MCP tool
            hook_side.db.execute("INSERT INTO questions (created_at, question, norm) VALUES ('t', ?, 'n')", (f"Q{i}",))
        assert stamp_of(migrated) == CURRENT_PRE_BRIDGE
        Store(migrated, migrate=True).close()  # the bridge worker's next tick
        assert stamp_of(migrated) == revision_stamp()
    with Store(migrated) as s:
        assert sorted(q.question for q in s.list()) == ["Q0", "Q1", "Q2"]


def test_concurrent_pre_bridge_restamps_bridge_workers_and_hooks(migrated):
    """The same, at once, from threads: never an error besides a busy database, never a
    SchemaNotReady for a hook (the schema never goes away), every row kept."""
    errors: list[str] = []
    written: list[int] = []
    stop = time.monotonic() + 1.5

    def guard(fn):
        def run():
            while time.monotonic() < stop:
                try:
                    fn()
                except sqlite3.OperationalError as exc:
                    if "locked" not in str(exc) and "busy" not in str(exc):
                        errors.append(repr(exc))
                except Exception as exc:  # noqa: BLE001
                    errors.append(repr(exc))
        return run

    def hook():
        with Store(migrated, timeout=0.5, maintenance=False) as s:
            cur = s.db.execute("INSERT INTO questions (created_at, question, norm) VALUES ('t', 'x', 'x')")
            written.append(cur.lastrowid)

    threads = [threading.Thread(target=guard(f)) for f in (
        lambda: pre_bridge_open(migrated, timeout=0.5),
        lambda: Store(migrated, timeout=0.5, migrate=True, maintenance=False).close(),
        hook)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert errors == []
    assert written
    with Store(migrated, migrate=True) as s:
        assert sorted(r[0] for r in s.db.execute("SELECT id FROM questions")) == sorted(written)


def test_two_bridge_revisions_interleave_forward_only(migrated, monkeypatch):
    """Revision 2 adds a column; a revision-1 worker still running never lowers its stamp, and
    a revision-2 hook does nothing until the revision-2 worker migrated."""
    with monkeypatch.context() as m:  # revision 2 is running: its hook comes first
        m.setattr(st, "MIGRATION_REVISION", 2)
        m.setitem(st.TABLE_ADDED_COLUMNS, "remote_outbox", {**st.TABLE_ADDED_COLUMNS["remote_outbox"],
                                                            "added_in_r2": "TEXT"})
        m.setattr(st, "_expected_cache", None)
        with pytest.raises(SchemaNotReady, match="added_in_r2"):
            Store(migrated, maintenance=False)
        assert stamp_of(migrated) == revision_stamp(1)
        Store(migrated, migrate=True).close()  # revision 2's worker
        assert stamp_of(migrated) == revision_stamp(2)
        assert "added_in_r2" in columns(migrated, "remote_outbox")
    st._expected_cache = None
    Store(migrated, migrate=True).close()  # the revision-1 worker, still running
    Store(migrated, maintenance=False).close()  # and its hooks
    assert stamp_of(migrated) == revision_stamp(2)
    assert "added_in_r2" in columns(migrated, "remote_outbox")  # never dropped


def test_the_cli_never_migrates(store, fake_herdr, capsys):
    """CLI commands open without migrating (store.py): on an older schema they say so and
    exit 1; herdr's startup sync-tokens exits 0 quietly; a remote export/apply answers with
    an error the pulling worker backs off on. The stamp and the schema are left alone."""
    import json

    from herdr_inbox import cli

    make_older_head(store.path)
    for argv in (["list"], ["answer", "1", "x"], ["dismiss", "1"], ["redeliver", "1"], ["deliver", "1"],
                 ["verify", "{}"]):
        assert cli.main(argv) == 1, argv
        assert "not migrated to this version yet" in capsys.readouterr().err
    assert cli.main(["sync-tokens"]) == 0
    assert capsys.readouterr().err == ""
    for command in ("export", "apply"):
        assert cli.main([command], stdin="{}") == 1
        assert json.loads(capsys.readouterr().out)["error"].startswith("not ready: ")
    assert stamp_of(store.path) == CURRENT_PRE_BRIDGE
    assert "sending_at" not in columns(store.path, "remote_outbox")


def test_the_worker_migrates_before_its_remote_thread_starts(fake_herdr, monkeypatch):
    """The worker is one of the two migrators, and migrates before the remote thread (which
    opens without migrating) can run; each tick's open migrates too (a pre-bridge server may
    have re-stamped meanwhile)."""
    from herdr_inbox import remote, worker

    path = st.db_path()
    Store(path, migrate=True).close()
    make_older_head(path)
    monkeypatch.setattr(worker, "TICK", 0.0)
    monkeypatch.setattr(remote, "enabled_machines", lambda ttl=60.0: [{"id": "m"}])
    at_start, at_tick = [], []
    monkeypatch.setattr(worker, "start_remote_thread", lambda: at_start.append(stamp_of(path)))

    def tick(store):
        at_tick.append(stamp_of(path))
        if len(at_tick) == 1:
            make_older_head(path, table="question_ext", column="legacy_checked_at")  # an old server, meanwhile
            return 0
        raise KeyboardInterrupt

    loops = []

    def installed_version():  # once per loop: a tick whose open fails must not loop forever here
        loops.append(1)
        if len(loops) > 3:
            raise KeyboardInterrupt
        return None

    monkeypatch.setattr(worker, "tick", tick)
    monkeypatch.setattr(worker, "installed_version", installed_version)
    with pytest.raises(KeyboardInterrupt):
        worker.run()
    assert at_start == [revision_stamp()] and at_tick == [revision_stamp()] * 2
    assert "legacy_checked_at" in columns(path, "question_ext")


def test_the_popup_migrates(fake_herdr):
    from herdr_inbox.tui import InboxApp

    path = st.db_path()
    Store(path, migrate=True).close()
    make_older_head(path)
    InboxApp().store.close()
    assert stamp_of(path) == revision_stamp() and "sending_at" in columns(path, "remote_outbox")
