"""0.4 processes keep running against a database a newer version has opened (review of #8):
read views are versioned and never dropped, so an older process's view stays put."""

import sqlite3
import threading
import time

from herdr_inbox import service, store as current
from herdr_inbox.service import Caller

from . import compat_v04_store as v04

A = Caller("w1:p1", "w1:t1", "tab", "sess-A", "/r")


def test_long_lived_v04_connection_survives_a_newer_open(tmp_path, fake_herdr):
    path = tmp_path / "inbox.db"
    old = v04.Store(path)  # a 0.4 popup or MCP server, connected before the update
    qid = old.insert("Old?", pane_id="w1:p1", session_id="sess-A").id
    new = current.Store(path)  # 0.5 opens (and migrates) the same DB
    service.ask_group(new, A, [{"question": "New one?"}, {"question": "Another?"}], title="Named")
    # the 0.4 connection keeps working: every query it makes goes through its own view
    assert sorted(q.id for q in old.list()) == [qid, 2, 3]
    assert old.get(2).question == "New one?" and old.group(2)[1].id == 3
    assert old.set_answer(qid, "fine") and [q.id for q in old.unacknowledged_answers("sess-A", "w1:p1")] == [qid]
    views = {r[0] for r in new.db.execute("SELECT name FROM sqlite_master WHERE type = 'view'")}
    assert {v04.VIEW, current.VIEW} <= views


def test_v04_and_newer_processes_opening_concurrently(tmp_path, fake_herdr):
    path = tmp_path / "inbox.db"
    current.Store(path).close()
    service.ask(current.Store(path), A, "Seed?")
    errors: list[str] = []
    stop = time.monotonic() + 1.5

    def loop(module):
        while time.monotonic() < stop:
            try:
                with module.Store(path) as s:
                    s.list()
                    s.get(1)
            except sqlite3.OperationalError as exc:
                if "locked" not in str(exc):
                    errors.append(f"{module.__name__}: {exc}")

    threads = [threading.Thread(target=loop, args=(m,)) for m in (v04, current)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert errors == []
