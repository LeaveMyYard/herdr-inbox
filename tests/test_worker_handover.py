"""herdr-inbox#50 end to end, with real processes, shaped like real installs (#75 reviews H1, M1).

Two install roots as herdr lays them out (``herdr-plugin.toml``, ``pyproject.toml``, ``hooks/``,
``src/herdr_inbox``; the newer one at MIGRATION_REVISION 2 with one more column, and a release's
``bin/herdr-inbox`` that has no worker yet). Every process runs as production runs it: the
workers as ensure() starts them, the hook as hooks.json does (``python3 -I hooks/run.py``), with
NO PYTHONPATH anywhere. An r1 worker holds the lock and keeps delivering; the only r2 processes
are hooks. The r2 worker must take over (stand by, then get the lock the r1 worker hands over),
the database must reach revision 2, and the r2 hooks must deliver.
"""

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

from herdr_inbox import revision_wanted, store as st, worker, worker_handover as wh
from herdr_inbox.store import Store, db_path

from .conftest import TEST_RUN_VAR, is_own_worker, leaked_workers, stop_own_workers

ROOT = Path(__file__).resolve().parents[1]
SESSION = "aaaaaaaa-0000-4000-8000-00000000000a"
LIMIT = 30.0  # every wait is bounded: a broken handover fails here, it never hangs
# The interpreter behind this venv: herdr_inbox is NOT installed in it, as in the system python3
# that hooks.json runs (a worker started from there must find its package on its own).
PYTHON = getattr(sys, "_base_executable", None) or sys.executable


def install(dest: Path, revision: int) -> Path:
    """An install root of this code at ``revision`` (a release's layout)."""
    dest.mkdir()
    for name in ("herdr-plugin.toml", "pyproject.toml"):
        shutil.copy(ROOT / name, dest / name)
    shutil.copytree(ROOT / "hooks", dest / "hooks", ignore=shutil.ignore_patterns("__pycache__"))
    shutil.copytree(ROOT / "src" / "herdr_inbox", dest / "src" / "herdr_inbox",
                    ignore=shutil.ignore_patterns("__pycache__"))
    if revision != 1:
        store = dest / "src" / "herdr_inbox" / "store.py"
        text, n = re.subn(r"^MIGRATION_REVISION = 1$", f"MIGRATION_REVISION = {revision}", store.read_text(),
                          flags=re.M)
        assert n == 1
        text = text.replace('"remote_outbox": {"sending_at": "TEXT"}',
                            '"remote_outbox": {"sending_at": "TEXT", "added_in_r2": "TEXT"}')
        assert "added_in_r2" in text
        store.write_text(text)
        # a release's binary before P5: no `worker`, prints its version and exits 0
        binary = dest / "bin" / "herdr-inbox"
        binary.parent.mkdir()
        binary.write_text("#!/bin/sh\necho 'herdr-inbox 0.10.6'\n")
        binary.chmod(0o755)
    return dest


@pytest.fixture
def world(migrated_db, tmp_path, monkeypatch):
    monkeypatch.delenv("HERDR_INBOX_NO_WORKER")
    monkeypatch.delenv("PYTHONPATH", raising=False)
    monkeypatch.setenv("HERDR_INBOX_WORKER_TICK", "0.2")
    monkeypatch.setenv("HERDR_INBOX_WORKER_IDLE_EXIT", "120")
    monkeypatch.setenv(wh.STANDBY_MAX_ENV, str(LIMIT))  # no standby outlives the test's wait
    monkeypatch.setenv("FAKE_HERDR_SESSION", SESSION)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")  # the r1 worker leaves the answer to the hooks
    listed = tmp_path / "listed-root"
    monkeypatch.setenv("FAKE_HERDR_INBOX_ROOT_FILE", str(listed))
    with Store(migrate=True) as s:
        q = s.insert("Which region?", session_id=SESSION, pane_id="w1:p1")
        s.set_answer(q.id, "eu")
    r1, r2 = install(tmp_path / "r1", 1), install(tmp_path / "r2", 2)
    listed.write_text(str(r1))
    procs: list[subprocess.Popen] = []
    yield {"r1": r1, "r2": r2, "listed": listed, "procs": procs, "qid": q.id}
    others = stop_own_workers(tmp_path, signal.SIGKILL)  # this run's own only, checked per pid
    for p in procs:
        p.kill()
        p.wait(timeout=10)
    deadline = time.monotonic() + 10
    while leaked_workers(tmp_path) and time.monotonic() < deadline:
        time.sleep(0.05)
    assert others == [], f"workers not started by this run, left alone: {others}"
    assert leaked_workers(tmp_path) == [], "no worker of this test outlives it"


def start_worker(world, root: Path) -> subprocess.Popen:
    """The worker of ``root`` as ensure() starts it (no PYTHONPATH: its src goes in argv)."""
    assert "PYTHONPATH" not in os.environ
    argv = [PYTHON, *worker.python_worker_argv(str(root / "src"))[1:]]
    p = subprocess.Popen(argv, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    world["procs"].append(p)
    return p


def hook(root: Path) -> str:
    """``root``'s delivery hook as hooks.json runs it: python3 -I hooks/run.py (run-hook.sh)."""
    assert "PYTHONPATH" not in os.environ
    out = subprocess.run([PYTHON, "-I", str(root / "hooks" / "run.py"), "post-tool-batch"],
                         input=json.dumps({"session_id": SESSION}), capture_output=True, text=True, timeout=60)
    return out.stdout


def stamp() -> int | None:
    import sqlite3

    db = sqlite3.connect(db_path())
    try:
        return st.stamped_revision(db.execute("PRAGMA user_version").fetchone()[0])
    finally:
        db.close()


def holder() -> int:
    return wh.holder_revision(worker._lock_path())


def wait_for(cond, limit: float = LIMIT) -> bool:
    deadline = time.monotonic() + limit
    while time.monotonic() < deadline:
        if cond():
            return True
        time.sleep(0.05)
    return cond()


def r2_took_over() -> bool:
    return holder() == 2 and stamp() == 2


def test_the_hook_interpreter_has_no_package_of_its_own():
    probe = subprocess.run([PYTHON, "-I", "-c", "import herdr_inbox"], capture_output=True, timeout=60)
    assert probe.returncode != 0, "PYTHON must be bare, or these tests prove less than production needs"


def test_r2_hooks_alone_get_an_r2_worker_while_r1_ensure_callers_fire(world):
    start_worker(world, world["r1"])
    assert wait_for(lambda: worker.is_running() and holder() == 1)
    stop = threading.Event()
    holders = []

    def r1_callers():  # the r1 MCP servers, popup, CLI: ensure() whenever they run
        while not stop.is_set():
            worker.ensure()
            holders.append(holder() if worker.is_running() else None)
            time.sleep(0.05)

    caller = threading.Thread(target=r1_callers, daemon=True)
    caller.start()
    try:
        assert hook(world["r2"]) == "", "not ready yet: nothing printed, an r2 worker started"
        assert wait_for(r2_took_over), (holder(), stamp(), (db_path().parent / "worker.log").read_text()
                                       if (db_path().parent / "worker.log").exists() else "")
        # the r1 callers keep firing after the takeover: ten more of their rounds, whatever the time
        after = len(holders)
        assert wait_for(lambda: len(holders) >= after + 10)
    finally:
        stop.set()
        caller.join(5)
    out = json.loads(hook(world["r2"]))["hookSpecificOutput"]["additionalContext"]
    assert f"Q{world['qid']} answered" in out, "the r2 hook delivers"
    # one handover, r1 to r2: once r2 holds, no r1 worker ever holds again (the free instant in
    # between is the design's one poll; its length is not asserted, a loaded machine stretches it)
    seen = [h for h in holders if h is not None]
    assert seen and seen[-1] == 2 and 1 not in seen[seen.index(2):], seen
    assert "Error" not in ((db_path().parent / "worker.log").read_text()
                           if (db_path().parent / "worker.log").exists() else "")


def asked_herdr_for_its_root() -> bool:
    """The worker has listed herdr's plugins (it decides then whether it is herdr's install)."""
    log = Path(os.environ["FAKE_HERDR_LOG"])
    return log.exists() and any(json.loads(line)[:2] == ["plugin", "list"] for line in log.read_text().splitlines())


def test_a_rust_hooks_marker_alone_gets_the_r2_worker_of_a_release_install(world):
    """#75 review M1: the Rust hooks only write the revision-wanted marker. herdr lists the r2
    install, whose bin/herdr-inbox has no worker yet; the r1 holder (no version handover: same
    version) must start r2's Python worker, which stands by and takes over."""
    start_worker(world, world["r1"])
    assert wait_for(lambda: worker.is_running() and holder() == 1 and asked_herdr_for_its_root())
    world["listed"].write_text(str(world["r2"]))  # herdr updated the install
    revision_wanted.want(db_path(), 2)  # what a not-ready Rust hook writes (src/store/revision_wanted.rs)
    assert wait_for(r2_took_over), (holder(), stamp())
    out = json.loads(hook(world["r2"]))["hookSpecificOutput"]["additionalContext"]
    assert f"Q{world['qid']} answered" in out


def test_a_marker_reaches_the_listed_install_from_a_holder_herdr_does_not_list(world, tmp_path):
    """#75 round 3 M2: the holder runs from a copy herdr never lists (the Claude-cache copy an MCP
    server or a hook started it from). The marker still gets herdr's listed r2 install started."""
    cache = install(tmp_path / "claude-cache-r1", 1)
    world["listed"].write_text(str(world["r2"]))
    start_worker(world, cache)
    assert wait_for(lambda: worker.is_running() and holder() == 1 and asked_herdr_for_its_root())
    revision_wanted.want(db_path(), 2)
    assert wait_for(r2_took_over), (holder(), stamp())


def test_the_newer_worker_wins_the_lock_race_whatever_the_start_order(world):
    from .compat_prebridge import set_stamp

    for round_ in range(3):
        set_stamp(db_path(), st.revision_stamp(1))  # every round starts at revision 1
        first, second = (world["r1"], world["r2"]) if round_ % 2 == 0 else (world["r2"], world["r1"])
        a, b = start_worker(world, first), start_worker(world, second)
        assert wait_for(r2_took_over), round_
        for p in (a, b):
            p.kill()
            p.wait(timeout=10)
        assert wait_for(lambda: not worker.is_running(), 10)


@pytest.fixture
def bystander(tmp_path):
    """A live stand-in for a worker of ANOTHER database (as the operator's own is): its
    directory sits beside this test's and shares its path as a prefix, with a worker lock and
    log of its own. Requested before ``world``, so it is torn down after it: the world's
    teardown must have left it running."""
    other = tmp_path.parent / f"{tmp_path.name}-other"
    other.mkdir()
    (other / "worker.log").write_text("")
    (other / "worker-0123456789ab.lock").write_text("")
    env = {k: v for k, v in os.environ.items() if k != "HERDR_INBOX_DB"}
    env["HERDR_INBOX_DB"] = str(other / "inbox.db")
    # its argv looks like a worker's: herdr_inbox.cli ... worker
    p = subprocess.Popen([sys.executable, "-c", "import time  # herdr_inbox.cli\ntime.sleep(120)", "worker"],
                         env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         start_new_session=True)
    try:
        assert wait_for(lambda: p.pid in leaked_workers(other), 10), "the stand-in matches as its own database's worker"
        yield p
        assert p.poll() is None, "the test's teardown killed another database's worker"
    finally:
        p.kill()
        p.wait(timeout=10)


def test_a_worker_started_without_ensure_is_found_and_stopped(bystander, world, tmp_path):
    """#84 review: workers a holder starts (_spawn_newer) write no worker.log; teardown must
    still find them by their database, and nothing else."""
    assert not (tmp_path / "worker.log").exists()
    p = start_worker(world, world["r1"])
    assert wait_for(lambda: worker.is_running())
    assert not (tmp_path / "worker.log").exists(), "started the way _spawn_newer starts one"
    found = leaked_workers(tmp_path)
    assert p.pid in found
    assert bystander.pid not in found, "a worker of another database is never listed"


def _outsider(tmp_path: Path) -> int:
    """A stand-in for a worker this run did NOT start (the operator's own): argv like a worker's,
    HERDR_INBOX_DB under ``tmp_path`` (so every database matcher lists it), but double-forked into
    its own session (not a descendant of this process) and without this run's mark."""
    (tmp_path / "worker.log").write_text("")
    pidfile = tmp_path / "outsider.pid"
    env = {k: v for k, v in os.environ.items() if k != TEST_RUN_VAR}
    env["HERDR_INBOX_DB"] = str(tmp_path / "inbox.db")
    code = ("import os, sys, time\n"
            "if os.fork(): os._exit(0)\n"
            "os.setsid()\n"
            "if os.fork(): os._exit(0)\n"
            "open(sys.argv[1] + '.tmp', 'w').write(str(os.getpid())); os.replace(sys.argv[1] + '.tmp', sys.argv[1])\n"
            "time.sleep(120)  # herdr_inbox.cli")
    subprocess.run([sys.executable, "-c", code, str(pidfile), "worker"], env=env, timeout=30, check=True)
    assert wait_for(pidfile.exists, 10)
    return int(pidfile.read_text())


def _alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def test_a_worker_this_run_did_not_start_is_never_signalled(tmp_path, monkeypatch):
    """The operator's live worker was killed once by a test whose matcher was broken (a mutation
    run). Whatever the matcher lists, a test signals only this run's own workers."""
    from . import conftest

    pid = _outsider(tmp_path)
    try:
        assert pid in leaked_workers(tmp_path), "the database matcher lists it"
        assert not is_own_worker(pid, tmp_path), "but it is not this run's"
        assert stop_own_workers(tmp_path) == [pid] and _alive(pid), "reported, never signalled"
        # a matcher broken to list it anyway (as the mutation did) still never gets it signalled
        monkeypatch.setattr(conftest, "leaked_workers", lambda root: [pid])
        assert conftest.stop_own_workers(tmp_path, signal.SIGKILL) == [pid]
        time.sleep(0.2)
        assert _alive(pid)
    finally:
        os.kill(pid, signal.SIGKILL)  # the test started it: its own pid, from its own pid file
        (tmp_path / "worker.log").unlink()
