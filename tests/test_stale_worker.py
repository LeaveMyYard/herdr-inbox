"""herdr-inbox#50 (Rust rewrite plan §5.4, a precondition for any MIGRATION_REVISION bump): a
worker of an older revision that keeps the worker lock must not leave a newer release's hooks
and MCP tools "not ready" until the popup is opened. The newer processes say which revision
they want (revision_wanted); the older worker makes way; the newer worker then migrates."""

import os
import time
from contextlib import contextmanager

import pytest

from herdr_inbox import revision_wanted, store as st, worker
from herdr_inbox.store import SchemaNotReady, Store, db_path, revision_stamp, stamped_revision


@contextmanager
def revision_2(monkeypatch):
    """This process runs revision 2, whose schema adds a column."""
    with monkeypatch.context() as m:
        m.setattr(st, "MIGRATION_REVISION", 2)
        m.setitem(st.TABLE_ADDED_COLUMNS, "remote_outbox",
                  {**st.TABLE_ADDED_COLUMNS["remote_outbox"], "added_in_r2": "TEXT"})
        m.setattr(st, "_expected_cache", None)
        yield
    st._expected_cache = None


def stamp() -> int | None:
    import sqlite3

    db = sqlite3.connect(db_path())
    try:
        return stamped_revision(db.execute("PRAGMA user_version").fetchone()[0])
    finally:
        db.close()


@pytest.fixture
def r1_worker(migrated_db, monkeypatch):
    """A revision-1 worker that would otherwise run on (always something pending, no update
    seen): it stops only by making way."""
    monkeypatch.setattr(worker, "TICK", 0.0)
    monkeypatch.setattr(worker, "IDLE_EXIT", 3600.0)
    monkeypatch.setattr(worker, "running_version", lambda: "0.10.6")
    monkeypatch.setattr(worker, "installed_version", lambda: "0.10.6")
    ticks = []

    def tick(store):
        ticks.append(1)
        if len(ticks) > 50:
            raise SystemExit("the revision-1 worker never made way")
        return 1

    monkeypatch.setattr(worker, "tick", tick)
    return ticks


def test_a_newer_open_on_an_older_schema_asks_for_its_revision(migrated_db, monkeypatch):
    with revision_2(monkeypatch):
        with pytest.raises(SchemaNotReady, match="added_in_r2"):
            Store(maintenance=False)
    assert revision_wanted.wanted(db_path()) == 2
    assert stamp() == 1, "asking writes no stamp"


def test_an_older_worker_holding_the_lock_makes_way_for_the_newer_revision(r1_worker, monkeypatch):
    """The test #50 asks for: an r1 worker holds the lock, r2 hooks find the DB not ready."""
    held = worker.hold_lock()  # the r1 worker is running ...
    assert held is not None
    with revision_2(monkeypatch):
        with pytest.raises(SchemaNotReady):  # ... and an r2 hook comes along
            Store(maintenance=False)
        assert worker.is_running(), "ensure() would start nobody: the lock is held"
    held.close()
    assert worker.run() == 0, "the r1 worker makes way at its next tick"
    assert worker.hold_lock() is not None, "and released the lock"
    with revision_2(monkeypatch):  # the r2 worker the next ensure() starts migrates ...
        Store(migrate=True).close()
        assert stamp() == 2
        Store(maintenance=False).close()  # ... and the r2 hooks are ready
    assert worker.outranked() is False, "done: the marker no longer counts"


def test_the_r1_worker_keeps_serving_without_a_newer_release(r1_worker):
    assert worker.outranked() is False
    with pytest.raises(SystemExit, match="never made way"):
        worker.run()


@pytest.mark.parametrize("case", ["stale", "same revision", "already migrated"])
def test_a_marker_that_does_not_outrank_this_worker_is_ignored(migrated_db, monkeypatch, case):
    if case == "stale":  # written by an r2 process long gone (a rollback)
        revision_wanted.want(db_path(), 2)
        old = time.time() - revision_wanted.REVISION_WANTED_TTL - 5
        os.utime(revision_wanted.path_for(db_path()), (old, old))
    elif case == "same revision":
        revision_wanted.want(db_path(), st.MIGRATION_REVISION)
    else:  # an r2 worker migrated already: an r1 worker runs on against it (never lowers it)
        revision_wanted.want(db_path(), 2)
        with revision_2(monkeypatch):
            Store(migrate=True).close()
    assert worker.outranked() is False


def test_the_marker_never_lowers_a_higher_revision(tmp_path):
    db = tmp_path / "inbox.db"
    revision_wanted.want(db, 3)
    revision_wanted.want(db, 2)
    assert revision_wanted.wanted(db) == 3
    assert [p.name for p in tmp_path.iterdir()] == [revision_wanted.NAME], "no temporary file left"
    revision_wanted.path_for(db).write_text("garbage")
    assert revision_wanted.wanted(db) is None
    assert revision_stamp(2) < 0
