"""herdr-inbox#50 end to end, with real processes (#75 review H1): an r1 worker holds the lock and
keeps delivering; the only r2 processes are hooks (no r2 MCP server, no popup); older ensure()
callers keep firing. The r2 worker must take over (stand by, then get the lock the r1 worker hands
over) and the r2 hooks must deliver, with no moment in which nobody holds the lock."""

import json
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from herdr_inbox import store as st, worker, worker_handover as wh
from herdr_inbox.store import Store, db_path

from .compat_prebridge import set_stamp
from .conftest import leaked_workers

ROOT = Path(__file__).resolve().parents[1]
SESSION = "aaaaaaaa-0000-4000-8000-00000000000a"


def r2_package(tmp_path: Path) -> Path:
    """The package at revision 2: its schema adds a column (as a real bump would)."""
    dest = tmp_path / "r2"
    shutil.copytree(ROOT / "src" / "herdr_inbox", dest / "herdr_inbox",
                    ignore=shutil.ignore_patterns("__pycache__"))
    store = dest / "herdr_inbox" / "store.py"
    text = store.read_text()
    text, n = re.subn(r"^MIGRATION_REVISION = 1$", "MIGRATION_REVISION = 2", text, flags=re.M)
    assert n == 1
    text = text.replace('"remote_outbox": {"sending_at": "TEXT"}', '"remote_outbox": {"sending_at": "TEXT", "added_in_r2": "TEXT"}')
    assert "added_in_r2" in text
    store.write_text(text)
    return dest


@pytest.fixture
def world(migrated_db, tmp_path, monkeypatch):
    monkeypatch.delenv("HERDR_INBOX_NO_WORKER")
    monkeypatch.setenv("HERDR_INBOX_WORKER_TICK", "0.2")
    monkeypatch.setenv("HERDR_INBOX_WORKER_IDLE_EXIT", "120")
    monkeypatch.setenv("FAKE_HERDR_SESSION", SESSION)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")  # the r1 worker leaves the answer to the hooks
    with Store(migrate=True) as s:
        q = s.insert("Which region?", session_id=SESSION, pane_id="w1:p1")
        s.set_answer(q.id, "eu")
    r2 = r2_package(tmp_path)
    procs: list[subprocess.Popen] = []
    yield {"r2": r2, "procs": procs, "qid": q.id}
    for pid in leaked_workers(tmp_path):
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    for p in procs:
        p.kill()
        p.wait(timeout=10)
    deadline = time.monotonic() + 10
    while leaked_workers(tmp_path) and time.monotonic() < deadline:
        time.sleep(0.05)


def env_for(package: Path) -> dict:
    env = dict(os.environ)
    env["PYTHONPATH"] = str(package)
    return env


def start_worker(world, package: Path) -> subprocess.Popen:
    p = subprocess.Popen([sys.executable, "-m", "herdr_inbox.cli", "worker"], env=env_for(package),
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         start_new_session=True)
    world["procs"].append(p)
    return p


def r2_hook(world) -> str:
    code = "import sys; from herdr_inbox import hook; hook.run('post-tool-batch')"
    out = subprocess.run([sys.executable, "-c", code], input=json.dumps({"session_id": SESSION}),
                         env=env_for(world["r2"]), capture_output=True, text=True, timeout=60)
    return out.stdout


def stamp() -> int | None:
    import sqlite3

    db = sqlite3.connect(db_path())
    try:
        return st.stamped_revision(db.execute("PRAGMA user_version").fetchone()[0])
    finally:
        db.close()


def wait_for(cond, limit: float) -> bool:
    deadline = time.monotonic() + limit
    while time.monotonic() < deadline:
        if cond():
            return True
        time.sleep(0.05)
    return cond()


def test_r2_hooks_alone_get_an_r2_worker_while_r1_ensure_callers_fire(world):
    start_worker(world, ROOT / "src")
    assert wait_for(lambda: worker.is_running() and wh.holder_revision(worker._lock_path()) == 1, 10)
    stop = threading.Event()
    gaps = []

    def r1_callers():  # the r1 MCP servers, popup, CLI: ensure() whenever they run
        while not stop.is_set():
            worker.ensure()
            if not worker.is_running():
                gaps.append(time.monotonic())
            time.sleep(0.05)

    caller = threading.Thread(target=r1_callers, daemon=True)
    caller.start()
    try:
        assert r2_hook(world) == "", "not ready yet: nothing printed, an r2 worker started"
        assert wait_for(lambda: wh.holder_revision(worker._lock_path()) == 2 and stamp() == 2, 30), \
            (wh.holder_revision(worker._lock_path()), stamp())
    finally:
        stop.set()
        caller.join(5)
    assert worker.is_running()
    out = json.loads(r2_hook(world))["hookSpecificOutput"]["additionalContext"]
    assert f"Q{world['qid']} answered" in out, "the r2 hook delivers"
    # the lock was free only for the instant of the handover, never for a polling interval twice
    assert len(gaps) <= 1, gaps


def test_the_newer_worker_wins_the_lock_race_whatever_the_start_order(world):
    for round_ in range(3):
        set_stamp(db_path(), st.revision_stamp(1))  # every round starts at revision 1
        first, second = (ROOT / "src", world["r2"]) if round_ % 2 == 0 else (world["r2"], ROOT / "src")
        a, b = start_worker(world, first), start_worker(world, second)
        assert wait_for(lambda: wh.holder_revision(worker._lock_path()) == 2 and stamp() == 2, 30), round_
        for p in (a, b):
            p.kill()
            p.wait(timeout=10)
        assert wait_for(lambda: not worker.is_running(), 10)
