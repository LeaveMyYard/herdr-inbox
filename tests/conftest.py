"""Shared fixtures: a fake herdr binary and a temp inbox database.

The fake records every argv as one JSON line and answers with canned JSON
shaped like herdr 0.9.3's real ``.result`` payloads. Behaviour is steered with
environment variables so subprocesses (stdio server, hook) see the same fake.
"""

from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import uuid
from pathlib import Path

import pytest

COMPOSER = Path(__file__).parent / "fixtures" / "composer"  # captured Claude Code input boxes (2.1.301)

FAKE_HERDR = r'''#!{python}
import json, os, sys
args = sys.argv[1:]
with open(os.environ["FAKE_HERDR_LOG"], "a") as fh:
    fh.write(json.dumps(args) + "\n")
cmd = " ".join(args[:2])
if cmd in os.environ.get("FAKE_HERDR_FAIL", "").split(","):
    sys.stderr.write(json.dumps({{"error": {{"code": "boom", "message": "fake failure"}}}}))
    sys.exit(1)
session = os.environ.get("FAKE_HERDR_SESSION", "aaaaaaaa-0000-4000-8000-00000000000a")
tabs_file = os.environ.get("FAKE_HERDR_TABS")  # JSON {{tab_id: label}}; renames persist there
tabs = json.load(open(tabs_file)) if tabs_file and os.path.exists(tabs_file) else {{}}
PANE_TAB = {{"w1:p1": "w1:t1", "w1:p2": "w1:t2", "w1:p3": "w1:t1"}}
if cmd == "pane get" and args[2] in os.environ.get("FAKE_HERDR_GONE_PANES", "").split(","):
    sys.stderr.write(json.dumps({{"error": {{"code": "pane_not_found", "message": "pane " + args[2] + " not found"}}}}))
    sys.exit(1)
elif cmd == "pane get":
    pane = {{"pane_id": args[2], "tab_id": PANE_TAB.get(args[2], "w1:t1"), "workspace_id": "w1", "agent": "claude"}}
    if session:
        pane["agent_session"] = {{"agent": "claude", "kind": "id", "source": "herdr:claude", "value": session}}
    result = {{"pane": pane, "type": "pane_info"}}
elif cmd == "tab get":
    result = {{"tab": {{"tab_id": args[2], "label": tabs.get(args[2], "my-tab"), "workspace_id": "w1"}}, "type": "tab_info"}}
elif cmd == "tab rename":
    tabs[args[2]] = args[3]
    if tabs_file:
        json.dump(tabs, open(tabs_file, "w"))
    result = {{"tab": {{"tab_id": args[2], "label": args[3]}}, "type": "tab_info"}}
elif cmd == "tab list":
    result = {{"tabs": [{{"tab_id": t, "label": l, "workspace_id": "w1"}} for t, l in tabs.items()], "type": "tab_list"}}
elif cmd == "workspace list":
    result = {{"workspaces": [{{"workspace_id": "w1"}}], "type": "workspace_list"}}
elif cmd == "plugin config-dir":
    if args[2] == "hhdebb.herdr-radar":
        print(os.environ.get("FAKE_HERDR_RADAR_CONFIG_DIR", "/nonexistent-config-dir"))
    else:
        print(os.environ.get("FAKE_HERDR_CONFIG_DIR", "/nonexistent-config-dir"))
    sys.exit(0)
elif cmd == "plugin list":
    ids = [p for p in os.environ.get("FAKE_HERDR_PLUGINS", "").split(",") if p]
    result = {{"plugins": [{{"plugin_id": p, "enabled": True, "name": p}} for p in ids]}}
    root_file = os.environ.get("FAKE_HERDR_INBOX_ROOT_FILE")  # the inbox's plugin_root, read per call
    if root_file:
        with open(root_file) as fh:
            result["plugins"].append({{"plugin_id": "leavemyyard.herdr-inbox", "enabled": True,
                                      "name": "herdr-inbox", "plugin_root": fh.read().strip()}})
elif cmd == "agent get":
    if os.environ.get("FAKE_HERDR_AGENT", "1") != "1" or args[2] in os.environ.get("FAKE_HERDR_GONE_PANES", "").split(","):
        sys.stderr.write(json.dumps({{"error": {{"code": "agent_not_found", "message": "not found"}}}}))
        sys.exit(1)
    agent = {{"pane_id": args[2], "agent": os.environ.get("FAKE_HERDR_AGENT_KIND", "claude"),
             "agent_status": os.environ.get("FAKE_HERDR_STATUS", "idle"),
             "focused": os.environ.get("FAKE_HERDR_FOCUSED", "0") == "1"}}
    if session:
        agent["agent_session"] = {{"agent": agent["agent"], "kind": "id", "source": "herdr:claude", "value": session}}
    result = {{"agent": agent}}
elif cmd == "agent prompt":
    # Where a real Claude pane would record the typed text (see herdr_inbox.transcript).
    # FAKE_HERDR_LAND: "user" (default; an idle prompt), "queue" (queued mid-turn), or
    # "none": herdr accepts the prompt but it never lands -- the 0.4.0 live bug.
    land = os.environ.get("FAKE_HERDR_LAND", "user")
    config = os.environ.get("CLAUDE_CONFIG_DIR")
    if config and session and land != "none":
        folder = os.path.join(config, "projects", "-fake-project")
        os.makedirs(folder, exist_ok=True)
        if land == "queue":
            record = {{"type": "queue-operation", "operation": "enqueue", "content": args[3]}}
        else:
            record = {{"type": "user", "message": {{"role": "user", "content": args[3]}}}}
        # FAKE_HERDR_LAND_SESSION: the conversation the pane really runs (after a Claude Code
        # session continuation herdr can keep reporting the old id)
        with open(os.path.join(folder, os.environ.get("FAKE_HERDR_LAND_SESSION", session) + ".jsonl"), "a") as fh:
            fh.write(json.dumps(record) + "\n")
    result = {{"type": "agent_prompted"}}
elif cmd == "pane read":
    if "ansi" in args:  # the composer check reads the ANSI screen: an empty Claude input box by default
        ansi = os.environ.get("FAKE_HERDR_SCREEN_ANSI")
        sys.stdout.write(ansi if ansi is not None else open({composer_empty!r}).read())
    else:
        print(os.environ.get("FAKE_HERDR_SCREEN", ""))  # the pane's visible text (plain)
    sys.exit(0)
elif cmd == "agent list":
    result = {{"agents": json.loads(os.environ.get("FAKE_HERDR_AGENTS", "[]")), "type": "agent_list"}}
elif cmd == "pane list":
    result = {{"panes": [{{"pane_id": p, "tab_id": t}} for p, t in PANE_TAB.items()], "type": "pane_list"}}
else:
    result = {{"type": "ok"}}
print(json.dumps({{"id": "cli:" + cmd.replace(" ", ":"), "result": result}}))
'''


class FakeHerdr:
    def __init__(self, tmp_path: Path):
        self.bin = tmp_path / "herdr"
        self.log = tmp_path / "herdr-calls.jsonl"
        self.bin.write_text(FAKE_HERDR.format(python=sys.executable, composer_empty=str(COMPOSER / "empty.ansi")))
        self.bin.chmod(0o755)
        self.log.touch()

    def calls(self) -> list[list[str]]:
        return [json.loads(line) for line in self.log.read_text().splitlines() if line]

    def calls_of(self, *prefix: str) -> list[list[str]]:
        return [c for c in self.calls() if c[: len(prefix)] == list(prefix)]

    def reset(self) -> None:
        self.log.write_text("")

    def focused(self) -> list[list[str]]:
        return self.calls_of("agent", "focus") or self.calls_of("pane", "focus")

    async def went_to_agent(self, app, timeout: float = 10.0) -> None:
        """Wait, inside ``app.run_test()``, until the popup's go-to-agent finished: herdr got the
        focus call and the popup exited (``InboxApp._focused``). The focus runs in a worker thread
        (#19), so a fixed ``pilot.pause()`` races it on slow runners (#28, #31), and
        ``workers.wait_for_complete()`` raises ``WorkerCancelled`` once that exit cancels the
        worker it ran in. Waits for the effect itself, with a deadline."""
        import asyncio
        import time

        deadline = time.monotonic() + timeout
        while not (self.focused() and not app.is_running):
            assert time.monotonic() < deadline, (
                f"no go-to-agent within {timeout}s: focus calls {self.focused()}, popup running={app.is_running}")
            await asyncio.sleep(0.01)

    async def focus_settled(self, app, timeout: float = 10.0) -> None:
        """Wait until no go-to-agent worker is pending or running (or the popup exited): for a
        sequence that may or may not go to the agent. See went_to_agent."""
        import asyncio
        import time

        from textual.worker import WorkerState

        deadline = time.monotonic() + timeout
        while app.is_running and any(w.group == "focus" and w.state in (WorkerState.PENDING, WorkerState.RUNNING)
                                     for w in app.workers):
            assert time.monotonic() < deadline, f"a go-to-agent worker still running after {timeout}s"
            await asyncio.sleep(0.01)


# leaked_workers runs in fake_herdr's teardown, before monkeypatch undoes a test's patches: a
# test that replaced subprocess.Popen (worker.subprocess IS subprocess) must not break it
_REAL_POPEN = subprocess.Popen

# This test run's mark: every process the suite starts inherits it (the environment is copied to
# every child, however far it is detached or reparented), no process outside the run can carry it.
# A test signals a worker only when its environment, read again right before the signal, carries
# it: the operator's own worker is never signalled, whatever a matcher returns.
# Not HERDR_* (each test scrubs those: _no_live_herdr). Fresh for every run: an inherited value
# (a nested pytest, a shell that kept one) is never reused. Only this very process may reuse its
# own, because pytest imports this file twice (as ``conftest`` and as ``tests.conftest``) and both
# must agree: the value names the process that made it.
TEST_RUN_VAR = "INBOX_TEST_RUN"


def _fresh_run_token() -> str:
    mine = os.environ.get(TEST_RUN_VAR, "")
    if mine.startswith(f"{os.getpid()}:"):
        return mine
    token = f"{os.getpid()}:{uuid.uuid4().hex}"
    os.environ[TEST_RUN_VAR] = token
    return token


TEST_RUN = _fresh_run_token()


def leaked_workers(root: Path) -> list[int]:
    """Live ``inbox worker`` processes whose database (HERDR_INBOX_DB) is under ``root``.

    Covers every Python worker however it was started (worker.python_worker_argv: runs
    ``herdr_inbox.cli`` with the argument ``worker``): by ensure(), by a holder's _spawn_newer,
    as a successor or by a test. Matched by its environment's HERDR_INBOX_DB, never by name: a
    worker of another database (the operator's) is never listed. Cheap when no worker ran: a
    worker always creates its lock file (``worker-<key>.lock``) next to the database, and ensure()
    writes worker.log there, so without either under ``root`` there is nothing to look for."""
    if not (root / "worker.log").exists() and next(root.rglob("worker-*.lock"), None) is None:
        return []
    marker = f"HERDR_INBOX_DB={root}{os.sep}"
    found = []
    if sys.platform.startswith("linux"):
        for d in Path("/proc").iterdir():
            if not d.name.isdigit() or int(d.name) == os.getpid():
                continue
            try:
                args = (d / "cmdline").read_bytes().split(b"\0")
                env = (d / "environ").read_bytes().split(b"\0")
            except OSError:
                continue
            if any(b"herdr_inbox.cli" in a for a in args) and b"worker" in args \
                    and any(e.decode(errors="replace").startswith(marker) for e in env):
                found.append(int(d.name))
    else:  # macOS: ps -E shows the environment of the user's own processes
        with _REAL_POPEN(["ps", "-Aww", "-E", "-o", "pid=,command="], stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, text=True) as ps:
            out = ps.communicate()[0]
        for line in out.splitlines():
            pid, _, command = line.strip().partition(" ")
            if "herdr_inbox.cli" in command and " worker" in command and marker in command and pid.isdigit():
                found.append(int(pid))
    return found


def _identity(pid: int) -> tuple[list[str], list[str]] | None:
    """Process ``pid``'s argv and environment, read now (None: gone or unreadable)."""
    if sys.platform.startswith("linux"):
        try:
            args = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
            env = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
        except OSError:
            return None
        return [a.decode(errors="replace") for a in args], [e.decode(errors="replace") for e in env]
    with _REAL_POPEN(["ps", "-ww", "-E", "-o", "command=", "-p", str(pid)], stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, text=True) as ps:
        out = ps.communicate()[0].strip()
    # macOS: the environment follows the command line; split on spaces (our values have none)
    return (out.split(), out.split()) if out else None


def is_own_worker(pid: int, root: Path) -> bool:
    """``pid`` is, right now, a worker this test run started for a database under ``root``:
    its argv is a worker's, its HERDR_INBOX_DB is under ``root`` and its environment carries
    this run's TEST_RUN. Only then may a test signal it."""
    got = _identity(pid)
    if got is None:
        return False
    args, env = got
    return (any("herdr_inbox.cli" in a for a in args) and "worker" in args
            and any(e.startswith(f"HERDR_INBOX_DB={root}{os.sep}") for e in env)
            and f"{TEST_RUN_VAR}={TEST_RUN}" in env)


def _pidfd_calls():
    """(open(pid) -> fd, send(fd, sig)) on Linux, else None. The os/signal functions when this
    Python has them; otherwise the system calls themselves (pidfd_open 434, pidfd_send_signal
    424: one number on every Linux architecture), as some builds lack the wrappers."""
    if not sys.platform.startswith("linux"):
        return None
    if hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"):
        return os.pidfd_open, signal.pidfd_send_signal
    import ctypes

    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long

    def check(r: int) -> int:
        if r < 0:
            err = ctypes.get_errno()
            raise OSError(err, os.strerror(err)) if err != 3 else ProcessLookupError(err, os.strerror(err))
        return r

    def open_fd(pid: int) -> int:
        return check(libc.syscall(434, ctypes.c_int(pid), ctypes.c_uint(0)))

    def send(fd: int, sig: int) -> None:
        check(libc.syscall(424, ctypes.c_int(fd), ctypes.c_int(sig), None, ctypes.c_uint(0)))

    return open_fd, send


def signal_verified(pid: int, sig: int, verify) -> bool:
    """Send ``sig`` to ``pid`` only if ``verify(pid)`` holds, and only to the very process that
    was verified. On Linux a pidfd pins the process first, so a pid reused after the check is
    never hit; elsewhere the check runs right before ``os.kill``. True if it was sent."""
    calls = _pidfd_calls()
    if calls is not None:
        open_fd, send = calls
        try:
            fd = open_fd(pid)
        except ProcessLookupError:
            return False
        except OSError:  # no pidfd support here: the check-then-kill fallback below
            fd = None
        if fd is not None:
            try:
                if not verify(pid):  # read AFTER the pidfd pinned it: this process, not a successor
                    return False
                send(fd, sig)
                return True
            except ProcessLookupError:
                return False
            finally:
                os.close(fd)
    if not verify(pid):
        return False
    try:
        os.kill(pid, sig)
        return True
    except ProcessLookupError:
        return False


def stop_own_workers(root: Path, sig: int = signal.SIGKILL) -> list[int]:
    """Signal the workers ``leaked_workers(root)`` lists that are this run's own (is_own_worker,
    checked again for each pid right before its signal). Returns the ones it found but did NOT
    signal: they are reported, never touched."""
    others = []
    for pid in leaked_workers(root):
        if not is_own_worker(pid, root):
            others.append(pid)
            continue
        signal_verified(pid, sig, lambda p: is_own_worker(p, root))
    return others


@pytest.fixture
def fake_herdr(tmp_path, monkeypatch, _no_live_herdr) -> FakeHerdr:
    fake = FakeHerdr(tmp_path)
    monkeypatch.setenv("HERDR_BIN_PATH", str(fake.bin))
    monkeypatch.setenv("FAKE_HERDR_LOG", str(fake.log))
    monkeypatch.setenv("HERDR_INBOX_DB", str(tmp_path / "inbox.db"))
    monkeypatch.setenv("HERDR_PANE_ID", "w1:p1")
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path / "claude"))  # transcripts the fake writes
    monkeypatch.setenv("HERDR_INBOX_NO_WORKER", "1")  # never spawn a real delivery worker
    from herdr_inbox import tui as _tui

    monkeypatch.setattr(_tui, "DETACHED_SEND", False)  # the popup delivers in its thread here
    monkeypatch.setattr(_tui, "ENABLE_BACKGROUND", False)  # side effects inline: deterministic tests
    # herdr's own config.toml (radar.py): never the real one
    monkeypatch.setenv("HERDR_INBOX_HERDR_CONFIG", str(tmp_path / "herdr-config" / "config.toml"))
    # plugin config dir: tab prefix off unless a test turns it on (tests/test_tabs.py)
    cfg = tmp_path / "plugin-config"
    cfg.mkdir()
    (cfg / "config.toml").write_text('tab_prefix = ""\n')
    monkeypatch.setenv("HERDR_PLUGIN_CONFIG_DIR", str(cfg))
    monkeypatch.setenv("FAKE_HERDR_CONFIG_DIR", str(cfg))
    from herdr_inbox import tabs

    tabs._config_dir.cache_clear()
    for var in ("CLAUDE_CODE_SESSION_ID", "HERDR_TAB_ID", "FAKE_HERDR_FAIL", "FAKE_HERDR_AGENT", "FAKE_HERDR_SESSION", "FAKE_HERDR_AGENT_KIND",
                "FAKE_HERDR_STATUS", "FAKE_HERDR_FOCUSED",
                "FAKE_HERDR_TABS", "FAKE_HERDR_LAND", "FAKE_HERDR_PLUGINS", "FAKE_HERDR_RADAR_CONFIG_DIR", "FAKE_HERDR_INBOX_ROOT_FILE",
                "FAKE_HERDR_SCREEN", "FAKE_HERDR_SCREEN_ANSI", "FAKE_HERDR_AGENTS", "FAKE_HERDR_LAND_SESSION", "FAKE_HERDR_GONE_PANES"):
        monkeypatch.delenv(var, raising=False)
    yield fake
    leaked = leaked_workers(tmp_path)
    if leaked:
        others = stop_own_workers(tmp_path, signal.SIGTERM)  # this run's own only
        pytest.fail(f"inbox worker process(es) {leaked} outlived the test (database under {tmp_path}); "
                    "a test that starts a real worker must stop it"
                    + (f"; not this run's, left alone: {others}" if others else ""), pytrace=False)


@pytest.fixture
def store(fake_herdr):
    from herdr_inbox.store import Store

    s = Store(migrate=True)  # as the worker (or the popup) leaves it: only they migrate
    yield s
    s.close()


@pytest.fixture
def migrated_db(fake_herdr):
    """The inbox DB as the worker (or the popup) leaves it: only they migrate (store.py), so a
    test of a non-migrating process (MCP tool, hook, CLI) starts from this."""
    from herdr_inbox.store import Store

    Store(migrate=True).close()


def subprocess_env(**extra: str) -> dict[str, str]:
    """Environment for a child process that must see the fake herdr."""
    env = {k: v for k, v in os.environ.items()}
    env.update(extra)
    return env


NO_HERDR = "/nonexistent/herdr-not-allowed-in-tests"


@pytest.fixture(autouse=True)
def _no_live_herdr(monkeypatch):
    """pytest started inside a herdr pane inherits HERDR_SOCKET_PATH, HERDR_PANE_ID, ...: a
    test without the fake would then reach the operator's LIVE herdr. Every HERDR_* is
    scrubbed and herdr itself points at a path that does not exist; fake_herdr (which runs
    after this) sets its own. tests/test_isolation.py fails if this scrub goes."""
    for var in [v for v in os.environ if v.startswith("HERDR_")]:
        monkeypatch.delenv(var)
    monkeypatch.setenv("HERDR_BIN_PATH", NO_HERDR)
    from herdr_inbox import herdr

    monkeypatch.setattr(herdr, "_is_herdrs_install", None)  # decided per process: per test here


@pytest.fixture(autouse=True)
def _armed_at_once(monkeypatch):
    """Logic tests press keys immediately; the real delays have their own tests
    (tests/test_real_timing.py, which undoes this fixture)."""
    from herdr_inbox import tui

    if hasattr(tui, "ARM_SECONDS"):
        monkeypatch.setattr(tui, "ARM_SECONDS", 0.0)
    if hasattr(tui, "ROW_ARM_SECONDS"):
        monkeypatch.setattr(tui, "ROW_ARM_SECONDS", 0.0)
    if hasattr(tui, "LIVE_TAB_LABELS"):  # tests/test_tab_labels.py turns live labels on
        monkeypatch.setattr(tui, "LIVE_TAB_LABELS", False)
        tui._tab_labels.clear()
