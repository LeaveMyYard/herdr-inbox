"""herdr-inbox#50 (Rust rewrite plan §5.4, a precondition for any MIGRATION_REVISION bump): a
worker of an older revision that holds the worker lock hands it over to a newer worker that is
already standing by, never to nobody (worker_handover.py), and a newer process that cannot start a
worker asks through the revision-wanted marker (revision_wanted.py). The end-to-end runs with real
processes are in tests/test_worker_handover.py."""

import os
import time

import pytest

from herdr_inbox import revision_wanted as rw, server, store as st, worker, worker_handover as wh
from herdr_inbox.store import SchemaNotReady, Store, db_path

from .compat_prebridge import set_stamp


@pytest.fixture
def r2(monkeypatch):
    """This process runs revision 2, whose schema adds a column."""
    monkeypatch.setattr(st, "MIGRATION_REVISION", 2)
    monkeypatch.setitem(st.TABLE_ADDED_COLUMNS, "remote_outbox",
                        {**st.TABLE_ADDED_COLUMNS["remote_outbox"], "added_in_r2": "TEXT"})
    monkeypatch.setattr(st, "_expected_cache", None)
    yield
    st._expected_cache = None


def lock():
    return worker._lock_path()


# -- the marker -------------------------------------------------------------------------------

def test_a_newer_open_on_an_older_schema_asks_for_its_revision(migrated_db, r2):
    with pytest.raises(SchemaNotReady, match="added_in_r2"):
        Store(maintenance=False)
    assert rw.wanted(db_path()) == 2
    assert rw.path_for(db_path()).name == db_path().name + ".revision-wanted", "per database file"
    revision, written = rw.path_for(db_path()).read_text().split()
    assert revision == "2" and abs(float(written) - time.time()) < 5, "<revision> <unix seconds>"


def test_the_marker_never_lowers_a_fresh_higher_revision_and_expires(tmp_path):
    db = tmp_path / "inbox.db"
    rw.want(db, 3, now=1000.0)
    rw.want(db, 2, now=1001.0)
    assert rw.wanted(db, now=1002.0) == 3
    assert rw.wanted(db, now=1000.0 + rw.REVISION_WANTED_TTL + 1) is None, "stale"
    rw.want(db, 2, now=1000.0 + rw.REVISION_WANTED_TTL + 2)
    assert rw.wanted(db, now=1000.0 + rw.REVISION_WANTED_TTL + 3) == 2, "a stale higher one no longer counts"
    rw.want(db, 4, now=5000.0)
    assert rw.wanted(db, now=5000.0 - rw.CLOCK_SKEW - 1) is None, "dated in the future: not trusted"
    assert sorted(p.name for p in tmp_path.iterdir()) == ["inbox.db.revision-wanted",
                                                          "inbox.db.revision-wanted.lock"]
    rw.path_for(db).write_text("garbage")
    assert rw.wanted(db) is None
    assert rw.wanted(tmp_path / "other.db") is None, "another database's marker is not this one's"


# -- the holder and the standby ---------------------------------------------------------------

def test_holder_revision_is_the_live_holders_or_1(tmp_path):
    lk = tmp_path / "worker-x.lock"
    assert wh.holder_revision(lk) == 1, "a worker from before the record"
    wh.write_holder(lk, 3)
    assert wh.holder_revision(lk) == 3
    (tmp_path / "worker-x.lock.holder").write_text("3 999999999\n")
    assert wh.holder_revision(lk) == 1, "its process is gone"


def test_a_standby_counts_while_alive_and_fresh(tmp_path):
    lk = tmp_path / "worker-x.lock"
    standby = tmp_path / "worker-x.lock.standby"
    standby.write_text(f"2 {os.getpid()} {time.time():.3f}\n")
    assert wh.live_standby(lk) == (2, os.getpid())
    standby.write_text(f"2 {os.getpid()} {time.time() - wh.STANDBY_FRESH - 1:.3f}\n")
    assert wh.live_standby(lk) is None, "its heartbeat stopped"
    standby.write_text(f"2 999999999 {time.time():.3f}\n")
    assert wh.live_standby(lk) is None, "its process is gone"


def test_a_standby_takes_the_lock_once_free_and_cleans_up(tmp_path):
    lk = tmp_path / "worker-x.lock"
    tries = []
    t = [0.0]

    def take():
        tries.append(1)
        return "LOCK" if len(tries) == 30 else None

    def sleep(s):
        t[0] += s

    assert wh.wait_as_standby(lk, 2, take, clock=lambda: t[0], sleep=sleep) == "LOCK"
    assert not (tmp_path / "worker-x.lock.standby").exists(), "its record goes with it"


def test_one_standby_is_enough(tmp_path):
    lk = tmp_path / "worker-x.lock"
    other = os.getppid()  # alive, not us
    (tmp_path / "worker-x.lock.standby").write_text(f"2 {other} {time.time():.3f}\n")
    assert wh.wait_as_standby(lk, 2, lambda: None, sleep=lambda s: None) is None
    assert (tmp_path / "worker-x.lock.standby").read_text().split()[1] == str(other), "never removes another's record"


# -- the holder's side ------------------------------------------------------------------------

def test_the_holder_hands_over_only_to_a_live_newer_standby(migrated_db, monkeypatch):
    path = wh._standby_path(lock())
    assert worker.handed_over() is False
    path.write_text(f"{st.MIGRATION_REVISION} {os.getppid()} {time.time():.3f}\n")
    assert worker.handed_over() is False, "not newer"
    path.write_text(f"{st.MIGRATION_REVISION + 1} {os.getppid()} {time.time():.3f}\n")
    assert worker.handed_over() is True


@pytest.mark.parametrize("case,expected", [
    ("none", False), ("newer", True), ("same revision on a legacy stamp", False),
    ("db already there", False), ("standby waiting", False)])
def test_a_marker_asks_the_holder_for_a_newer_worker(migrated_db, case, expected):
    if case == "newer":
        rw.want(db_path(), st.MIGRATION_REVISION + 1)
    elif case == "same revision on a legacy stamp":  # want == own: never (even on an unstamped DB)
        rw.want(db_path(), st.MIGRATION_REVISION)
        set_stamp(db_path(), 12345)
    elif case == "db already there":
        rw.want(db_path(), st.MIGRATION_REVISION + 1)
        set_stamp(db_path(), st.revision_stamp(st.MIGRATION_REVISION + 1))
    elif case == "standby waiting":
        rw.want(db_path(), st.MIGRATION_REVISION + 1)
        wh._standby_path(lock()).write_text(f"{st.MIGRATION_REVISION + 1} {os.getppid()} {time.time():.3f}\n")
    assert worker._newer_wanted() is expected


def test_ensure_starts_a_standby_only_for_an_older_holder(migrated_db, monkeypatch, r2):
    monkeypatch.delenv("HERDR_INBOX_NO_WORKER")
    spawned = []
    monkeypatch.setattr(worker.subprocess, "Popen", lambda argv, **k: spawned.append(argv))
    held = worker.hold_lock()
    try:
        wh.write_holder(lock(), 2)
        assert worker.ensure() is False and spawned == [], "the holder is this revision"
        wh.write_holder(lock(), 1)
        assert worker.ensure() is True and spawned[-1][-2:] == ["herdr_inbox.cli", "worker"]
        wh._standby_path(lock()).write_text(f"2 {os.getppid()} {time.time():.3f}\n")
        assert worker.ensure() is False and len(spawned) == 1, "one stands by already"
    finally:
        held.close()


def test_a_tool_on_a_not_migrated_db_keeps_asking_for_its_worker_longer_than_a_tick(monkeypatch):
    """#75 review M1: the first call after an upgrade must not answer "unavailable" after one
    ensure() made while the older worker still held the lock."""
    ensured, slept = [], []
    monkeypatch.setattr(server, "_ensure_worker", lambda: ensured.append(1))
    monkeypatch.setattr(server.time, "sleep", lambda s: slept.append(s))

    @server.busy_safe
    def tool():
        raise SchemaNotReady("not yet")

    assert tool() == dict(server.service.NOT_MIGRATED)
    assert len(ensured) == len(server.NOT_READY_BACKOFF) + 1, "every attempt"
    assert sum(slept) > worker.TICK + 2, "longer than a tick plus a handover"
