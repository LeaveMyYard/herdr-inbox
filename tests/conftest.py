"""Shared fixtures: a fake herdr binary and a temp inbox database.

The fake records every argv as one JSON line and answers with canned JSON
shaped like herdr 0.9.3's real ``.result`` payloads. Behaviour is steered with
environment variables so subprocesses (stdio server, hook) see the same fake.
"""

from __future__ import annotations

import json
import os
import sys
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
session = os.environ.get("FAKE_HERDR_SESSION", "sess-A")
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
                "FAKE_HERDR_TABS", "FAKE_HERDR_LAND", "FAKE_HERDR_PLUGINS", "FAKE_HERDR_RADAR_CONFIG_DIR",
                "FAKE_HERDR_SCREEN", "FAKE_HERDR_SCREEN_ANSI", "FAKE_HERDR_AGENTS", "FAKE_HERDR_LAND_SESSION", "FAKE_HERDR_GONE_PANES"):
        monkeypatch.delenv(var, raising=False)
    return fake


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
