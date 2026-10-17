"""P0b (Rust rewrite plan §5.4 "Worker handover"): start_successor starts the Rust worker when
one is installed in the checkout, the Python worker otherwise.

The Rust worker takes the same worker-<key>.lock as the Python one, so the two are never both
live. There is no Rust code yet: a fake executable stands in for `herdr-inbox worker`. It
logs its argv and cwd, then takes the real lock (or not, per FAKE_RUST_MODE)."""

import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import pytest

from herdr_inbox import herdr, worker

FAKE_RUST = r'''#!{python}
import fcntl, json, os, sys, time
mode = os.environ.get("FAKE_RUST_MODE", "lock")
with open(os.environ["FAKE_RUST_LOG"], "a") as fh:
    fh.write(json.dumps({{"argv": sys.argv[1:], "cwd": os.getcwd(), "path": sys.argv[0],
                         "uv": sorted(k for k in os.environ if k.startswith("UV_") or k == "VIRTUAL_ENV")}}) + "\n")
if mode == "exit":  # a hooks-only binary: no `worker` subcommand
    sys.stderr.write("error: unrecognized subcommand 'worker'\n")
    sys.exit(2)
if mode == "hang":  # never takes the lock
    time.sleep(60)
    sys.exit(0)
lock = open(os.environ["FAKE_RUST_LOCK"], "a")
try:
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
except OSError:
    sys.exit(0)  # another worker runs: exit, as the Python worker does
time.sleep(float(os.environ.get("FAKE_RUST_HOLD", "60")))
'''


@pytest.fixture
def checkout(tmp_path, fake_herdr, monkeypatch):
    """A plugin checkout with no Rust binary yet, uv spawns recorded instead of run, and the
    fake Rust worker's log/lock wired up. Kills any fake still running afterwards."""
    root = tmp_path / "checkout"
    root.mkdir()
    monkeypatch.setattr(herdr, "current_plugin_root", lambda: str(root))
    log = tmp_path / "rust.log"
    monkeypatch.setenv("FAKE_RUST_LOG", str(log))
    monkeypatch.setenv("FAKE_RUST_LOCK", str(worker._lock_path()))
    monkeypatch.setattr(worker, "RUST_HANDOVER_WAIT", 3.0)
    real_popen = subprocess.Popen
    uv, procs = [], []

    def popen(argv, *a, **k):
        if argv[0] == "uv":
            uv.append((argv, k))
            return None
        proc = real_popen(argv, *a, **k)
        procs.append(proc)
        return proc

    monkeypatch.setattr(worker.subprocess, "Popen", popen)
    state = {"root": root, "log": log, "uv": uv, "procs": procs}
    yield state
    for proc in procs:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
        proc.wait(timeout=5)


def install(root: Path, rel: str = "target/release/herdr-inbox", mode: int = 0o755) -> Path:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(FAKE_RUST.format(python=sys.executable))
    path.chmod(mode)
    return path


def calls(log: Path) -> list[dict]:
    return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []


class StillLooping(BaseException):
    """Not an Exception: the worker loop catches those from tick() and keeps going."""


def bounded_tick(limit: int = 20, each=None):
    """A worker tick that always has something pending, and fails the test (instead of
    hanging it) if the worker is still looping after ``limit`` ticks."""
    ticks = []

    def tick(store):
        ticks.append(1)
        if each:
            each(len(ticks))
        if len(ticks) > limit:
            raise StillLooping(f"the worker still runs after {limit} ticks")
        return 1

    tick.ticks = ticks
    return tick


UV_WORKER = ["uv", "run", "--quiet", "--frozen", "--no-dev", "--project"]


def test_no_binary_starts_the_python_worker_exactly_as_before(checkout, monkeypatch):
    monkeypatch.setenv("UV_PROJECT_ENVIRONMENT", "/data/venv-0.10.5")
    assert worker.rust_worker_binary(str(checkout["root"])) is None
    assert worker.start_successor() is True
    [(argv, kwargs)] = checkout["uv"]
    assert argv == [*UV_WORKER, str(checkout["root"]), "inbox", "worker"]
    assert kwargs["start_new_session"] and kwargs["close_fds"] and "cwd" not in kwargs
    assert "UV_PROJECT_ENVIRONMENT" not in kwargs["env"]
    assert checkout["procs"] == [] and calls(checkout["log"]) == []


def test_an_installed_rust_worker_takes_over_the_lock_and_no_python_starts(checkout, monkeypatch):
    monkeypatch.setenv("UV_PROJECT_ENVIRONMENT", "/data/venv-0.10.5")
    binary = install(checkout["root"])
    assert worker.rust_worker_binary(str(checkout["root"])) == str(binary)
    assert worker.start_successor() is True
    assert worker.is_running() and checkout["uv"] == []
    [call] = calls(checkout["log"])
    assert call["argv"] == ["worker"] and call["cwd"] == str(checkout["root"]) and call["uv"] == []
    assert worker.hold_lock() is None  # the Rust worker holds it: a Python worker cannot start
    assert worker.run() == 0  # ...and one that tries exits at once


def test_target_release_is_preferred_over_bin(checkout):
    install(checkout["root"], "bin/herdr-inbox")
    assert worker.rust_worker_binary(str(checkout["root"])) == str(checkout["root"] / "bin/herdr-inbox")
    built = install(checkout["root"])
    assert worker.rust_worker_binary(str(checkout["root"])) == str(built)


@pytest.mark.parametrize("what", ["not-executable", "directory", "dangling-symlink"])
def test_only_an_executable_file_counts_as_installed(checkout, what):
    path = checkout["root"] / "target/release/herdr-inbox"
    if what == "not-executable":
        install(checkout["root"], mode=0o644)
    elif what == "directory":
        path.mkdir(parents=True)
    else:
        path.parent.mkdir(parents=True)
        path.symlink_to(checkout["root"] / "nowhere")
    assert worker.rust_worker_binary(str(checkout["root"])) is None
    assert worker.start_successor() is True
    assert [argv[0] for argv, _ in checkout["uv"]] == ["uv"] and checkout["procs"] == []


def test_a_symlinked_binary_counts(checkout):
    real = install(checkout["root"], "elsewhere/herdr-inbox")
    link = checkout["root"] / "target/release/herdr-inbox"
    link.parent.mkdir(parents=True)
    link.symlink_to(real)
    assert worker.rust_worker_binary(str(checkout["root"])) == str(link)


def test_a_binary_without_a_worker_falls_back_to_python_at_once(checkout, monkeypatch):
    """The hooks-only binary of P2 exits 2 on `worker`: the Python worker takes over, without
    waiting out RUST_HANDOVER_WAIT."""
    monkeypatch.setenv("FAKE_RUST_MODE", "exit")
    monkeypatch.setattr(worker, "RUST_HANDOVER_WAIT", 30.0)
    install(checkout["root"])
    started = time.monotonic()
    assert worker.start_successor() is True
    assert time.monotonic() - started < 10
    assert [c["argv"] for c in calls(checkout["log"])] == [["worker"]]
    assert [argv for argv, _ in checkout["uv"]] == [[*UV_WORKER, str(checkout["root"]), "inbox", "worker"]]


def test_a_binary_that_never_takes_the_lock_is_killed_and_python_starts(checkout, monkeypatch):
    monkeypatch.setenv("FAKE_RUST_MODE", "hang")
    monkeypatch.setattr(worker, "RUST_HANDOVER_WAIT", 0.5)
    install(checkout["root"])
    assert worker.start_successor() is True
    [proc] = checkout["procs"]
    assert proc.poll() is not None  # killed, not left behind
    assert len(checkout["uv"]) == 1


def test_a_rust_worker_that_finds_the_lock_taken_still_counts_as_handed_over(checkout):
    """Between the release and the spawn another worker (e.g. ensure() from an MCP server)
    took the lock: the Rust worker exits, someone serves, no Python successor is added."""
    held = worker.hold_lock()
    try:
        install(checkout["root"])
        assert worker.start_successor() is True
        [proc] = checkout["procs"]
        assert proc.wait(timeout=10) == 0  # it exited: never both live
        assert checkout["uv"] == [] and len(calls(checkout["log"])) == 1
    finally:
        held.close()


def test_exec_format_error_falls_back_to_python(checkout):
    """A binary for another platform (ENOEXEC) is an OSError at spawn: Python takes over."""
    path = checkout["root"] / "target/release/herdr-inbox"
    path.parent.mkdir(parents=True)
    path.write_bytes(b"\x7fELF\x02garbage")
    path.chmod(0o755)
    assert worker.start_successor() is True
    assert len(checkout["uv"]) == 1


def test_the_running_worker_hands_over_to_rust_and_exits(checkout, monkeypatch):
    """The whole handover: a Python worker sees a new version on disk, releases the lock, the
    Rust worker takes it, the Python worker returns. At no point do both hold it."""
    monkeypatch.setattr(worker, "IDLE_EXIT", 3600.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    monkeypatch.setattr(worker, "running_version", lambda: "0.10.5")
    monkeypatch.setattr(worker, "installed_version", lambda: "0.11.0")
    monkeypatch.setattr(worker, "tick", bounded_tick())
    install(checkout["root"])
    assert worker.run() == 0
    assert worker.is_running() and checkout["uv"] == []
    assert worker.hold_lock() is None



# -- plan amendment (panorama #50, §4 item 5): herdr's current plugin_root, and no worker
#    outlives its herdr server ---------------------------------------------------------------

FAKE_HERDR = r"""#!{python}
import json, os, sys
args = sys.argv[1:]
with open(os.environ["FAKE_HERDR_LOG"], "a") as fh:
    fh.write(json.dumps(args) + "\n")
state = os.environ.get("FAKE_HERDR_STATE", "up")
if state == "gone":
    sock = os.environ.get("HERDR_SOCKET_PATH", "?")
    sys.stderr.write(json.dumps({{"id": "cli:" + ":".join(args[:2]), "error": {{"code": "server_not_running",
                     "message": "no herdr server is running at " + sock}}}}))
    sys.exit(1)
if state == "other-error":
    sys.stderr.write(json.dumps({{"error": {{"code": "timeout", "message": "slow"}}}}))
    sys.exit(1)
if state == "not-json":
    sys.stderr.write("herdr: something odd")
    sys.exit(1)
if args[:2] == ["plugin", "list"]:
    root_file = os.environ.get("FAKE_HERDR_ROOT_FILE")
    root = open(root_file).read().strip() if root_file and os.path.exists(root_file) else ""
    plugins = [{{"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": root}}] if root else []
    print(json.dumps({{"result": {{"plugins": plugins}}}}))
else:
    print(json.dumps({{"result": {{"workspaces": [], "agents": [], "panes": []}}}}))
"""


@pytest.fixture
def herdr_cli(tmp_path, monkeypatch):
    """A fake herdr CLI (HERDR_* of the test session are already unset by conftest)."""
    path = tmp_path / "fake-herdr"
    path.write_text(FAKE_HERDR.format(python=sys.executable))
    path.chmod(0o755)
    log = tmp_path / "herdr.log"
    root_file = tmp_path / "listed-root"
    monkeypatch.setenv("HERDR_BIN_PATH", str(path))
    monkeypatch.setenv("FAKE_HERDR_LOG", str(log))
    monkeypatch.setenv("FAKE_HERDR_ROOT_FILE", str(root_file))
    monkeypatch.setenv("HERDR_INBOX_DB", str(tmp_path / "inbox.db"))
    monkeypatch.setenv("HERDR_SOCKET_PATH", str(tmp_path / "herdr.sock"))
    monkeypatch.setenv("HERDR_INBOX_NO_WORKER", "1")
    return {"root_file": root_file, "log": log}


def listed_install(tmp_path: Path, name: str, version: str) -> Path:
    root = tmp_path / "installs" / name
    (root / "src" / "herdr_inbox").mkdir(parents=True)
    (root / "herdr-plugin.toml").write_text(f'version = "{version}"\n')
    (root / "pyproject.toml").write_text(f'[project]\nname = "herdr-inbox"\nversion = "{version}"\n')
    return root


def test_current_plugin_root_asks_herdr_every_time(tmp_path, herdr_cli):
    """herdr 0.9.4+ installs each update in a new directory: the lookup is herdr-first and
    uncached, so a long-lived worker sees the new root."""
    own = herdr._own_checkout()
    assert own and herdr.current_plugin_root() == own  # not listed: this code's checkout
    first, second = listed_install(tmp_path, "a", "1.0.0"), listed_install(tmp_path, "b", "1.1.0")
    herdr_cli["root_file"].write_text(str(first))
    assert herdr.current_plugin_root() == str(first)  # herdr wins over __file__
    herdr_cli["root_file"].write_text(str(second))
    assert herdr.current_plugin_root() == str(second)  # not cached
    assert worker.installed_version() == "1.1.0"


def test_the_worker_hands_over_to_the_new_install_directory(tmp_path, herdr_cli, monkeypatch):
    """The update landed in a new directory with a Rust binary: the worker sees the version
    there and starts that directory's binary, from that directory."""
    new = listed_install(tmp_path, "0.11.0", "0.11.0")
    install(new, "bin/herdr-inbox")
    herdr_cli["root_file"].write_text(str(new))
    rust_log = tmp_path / "rust.log"
    monkeypatch.setenv("FAKE_RUST_LOG", str(rust_log))
    monkeypatch.setenv("FAKE_RUST_LOCK", str(worker._lock_path()))
    monkeypatch.setattr(worker, "running_version", lambda: "0.10.5")
    monkeypatch.setattr(worker, "tick", bounded_tick())
    monkeypatch.setattr(worker, "TICK", 0.0)
    real_popen = subprocess.Popen
    procs = []
    monkeypatch.setattr(worker.subprocess, "Popen", lambda *a, **k: procs.append(real_popen(*a, **k)) or procs[-1])
    try:
        assert worker.run() == 0
        assert worker.is_running()
        [call] = calls(rust_log)
        assert call["path"] == str(new / "bin/herdr-inbox") and call["cwd"] == str(new)
    finally:
        for proc in procs:
            proc.kill()
            proc.wait(timeout=5)


@pytest.mark.parametrize("state,gone", [("gone", True), ("up", False), ("other-error", False), ("not-json", False)])
def test_server_gone_needs_herdrs_own_word(herdr_cli, monkeypatch, state, gone):
    monkeypatch.setenv("FAKE_HERDR_STATE", state)
    assert herdr.server_gone() is gone


def test_server_gone_without_a_herdr_binary_is_not_gone(herdr_cli, monkeypatch, tmp_path):
    monkeypatch.setenv("HERDR_BIN_PATH", str(tmp_path / "no-such-herdr"))
    assert herdr.server_gone() is False


def test_the_worker_exits_when_its_server_is_gone(herdr_cli, monkeypatch):
    """worker.log grew to 11 MB from workers looping on server_not_running for servers that
    had long exited. A detached worker must not outlive its server: it exits and releases
    the lock, without ticking."""
    monkeypatch.setenv("FAKE_HERDR_STATE", "gone")
    monkeypatch.setattr(worker, "IDLE_EXIT", 3600.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    tick = bounded_tick()  # answers pending
    monkeypatch.setattr(worker, "tick", tick)
    assert worker.run() == 0
    assert tick.ticks == [] and not worker.is_running()


def test_the_worker_exits_once_the_server_goes_away(herdr_cli, monkeypatch):
    monkeypatch.setattr(worker, "IDLE_EXIT", 3600.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    monkeypatch.setenv("FAKE_HERDR_STATE", "up")

    def server_exits(n):
        if n == 3:
            os.environ["FAKE_HERDR_STATE"] = "gone"

    tick = bounded_tick(each=server_exits)
    monkeypatch.setattr(worker, "tick", tick)
    assert worker.run() == 0
    assert len(tick.ticks) == 3 and not worker.is_running()


@pytest.mark.parametrize("state", ["other-error", "not-json"])
def test_other_herdr_failures_keep_the_worker_running(herdr_cli, monkeypatch, state):
    monkeypatch.setenv("FAKE_HERDR_STATE", state)
    monkeypatch.setattr(worker, "IDLE_EXIT", 3600.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    ticks = []

    def tick(store):
        ticks.append(1)
        if len(ticks) >= 3:
            raise KeyboardInterrupt  # stop the test loop: the worker kept running
        return 1

    monkeypatch.setattr(worker, "tick", tick)
    with pytest.raises(KeyboardInterrupt):
        worker.run()
    assert len(ticks) == 3
