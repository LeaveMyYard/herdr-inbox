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
if cmd == "pane get":
    pane = {{"pane_id": args[2], "tab_id": "w1:t1", "workspace_id": "w1", "agent": "claude"}}
    if session:
        pane["agent_session"] = {{"agent": "claude", "kind": "id", "source": "herdr:claude", "value": session}}
    result = {{"pane": pane, "type": "pane_info"}}
elif cmd == "tab get":
    result = {{"tab": {{"tab_id": args[2], "label": "my-tab", "workspace_id": "w1"}}, "type": "tab_info"}}
elif cmd == "agent get":
    if os.environ.get("FAKE_HERDR_AGENT", "1") != "1":
        sys.stderr.write(json.dumps({{"error": {{"code": "agent_not_found", "message": "not found"}}}}))
        sys.exit(1)
    result = {{"agent": {{"pane_id": args[2], "agent": "claude", "agent_status": "working"}}}}
elif cmd == "pane list":
    result = {{"panes": [{{"pane_id": "w1:p1"}}, {{"pane_id": "w1:p2"}}], "type": "pane_list"}}
else:
    result = {{"type": "ok"}}
print(json.dumps({{"id": "cli:" + cmd.replace(" ", ":"), "result": result}}))
'''


class FakeHerdr:
    def __init__(self, tmp_path: Path):
        self.bin = tmp_path / "herdr"
        self.log = tmp_path / "herdr-calls.jsonl"
        self.bin.write_text(FAKE_HERDR.format(python=sys.executable))
        self.bin.chmod(0o755)
        self.log.touch()

    def calls(self) -> list[list[str]]:
        return [json.loads(line) for line in self.log.read_text().splitlines() if line]

    def calls_of(self, *prefix: str) -> list[list[str]]:
        return [c for c in self.calls() if c[: len(prefix)] == list(prefix)]

    def reset(self) -> None:
        self.log.write_text("")


@pytest.fixture
def fake_herdr(tmp_path, monkeypatch) -> FakeHerdr:
    fake = FakeHerdr(tmp_path)
    monkeypatch.setenv("HERDR_BIN_PATH", str(fake.bin))
    monkeypatch.setenv("FAKE_HERDR_LOG", str(fake.log))
    monkeypatch.setenv("HERDR_INBOX_DB", str(tmp_path / "inbox.db"))
    monkeypatch.setenv("HERDR_PANE_ID", "w1:p1")
    for var in ("CLAUDE_CODE_SESSION_ID", "HERDR_TAB_ID", "FAKE_HERDR_FAIL", "FAKE_HERDR_AGENT", "FAKE_HERDR_SESSION"):
        monkeypatch.delenv(var, raising=False)
    return fake


@pytest.fixture
def store(fake_herdr):
    from herdr_inbox.store import Store

    s = Store()
    yield s
    s.close()


def subprocess_env(**extra: str) -> dict[str, str]:
    """Environment for a child process that must see the fake herdr."""
    env = {k: v for k, v in os.environ.items()}
    env.update(extra)
    return env
