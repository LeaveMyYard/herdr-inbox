"""Claude Code plugin hooks: output shape, compact/resume listing, never-fail."""

import json
import subprocess
import sys
from pathlib import Path

from herdr_inbox import service
from herdr_inbox.hook import SESSION_RULES
from herdr_inbox.service import Caller

from .conftest import subprocess_env

ROOT = Path(__file__).resolve().parents[1]


def run_hook(event, payload, **env):
    proc = subprocess.run(
        [sys.executable, "-m", "herdr_inbox.cli", "hook", event],
        input=payload if isinstance(payload, str) else json.dumps(payload),
        capture_output=True, text=True, timeout=30, env=subprocess_env(**env),
    )
    return proc


def context_of(proc, event="SessionStart"):
    assert proc.returncode == 0, proc.stderr
    out = json.loads(proc.stdout)
    assert list(out) == ["hookSpecificOutput"]
    assert out["hookSpecificOutput"]["hookEventName"] == event
    return out["hookSpecificOutput"]["additionalContext"]


def seed(store, monkeypatch):
    c = Caller(pane_id="w1:p1", tab_id="w1:t1", tab_label="t", session_id="sess-A", cwd="/r")
    service.ask(store, c, "Which region?")
    answered = service.ask(store, c, "Squash or merge?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")  # delivered answers count as seen
    service.answer(store, answered, "squash")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    service.ask(store, Caller("w1:p2", None, None, "sess-B", "/r"), "Other session question?")


def test_rules_are_short():
    assert len(SESSION_RULES.split()) < 250


def test_startup_injects_rules(fake_herdr):
    ctx = context_of(run_hook("session-start", {"session_id": "sess-A", "source": "startup"}))
    assert ctx == SESSION_RULES
    for phrase in ("ask_operator", "Ask once", "`[inbox]` messages naming the Q ids", "list_my_questions",
                   "destructive", "subagent", "herdr-inbox:operator-questions"):
        assert phrase in ctx


def test_startup_does_not_list_questions(store, fake_herdr, monkeypatch):
    seed(store, monkeypatch)
    ctx = context_of(run_hook("session-start", {"session_id": "sess-A", "source": "startup"}))
    assert "Q1" not in ctx


def test_compact_lists_this_sessions_pending_questions(store, fake_herdr, monkeypatch):
    seed(store, monkeypatch)
    ctx = context_of(run_hook("session-start", {"session_id": "sess-A", "source": "compact"}))
    assert ctx.startswith(SESSION_RULES)
    assert "Q1 (open): Which region?" in ctx
    assert "Q2 (answered, not yet seen): Squash or merge? -> squash" in ctx
    assert "Other session question" not in ctx


def test_compact_without_questions_is_just_the_rules(store, fake_herdr):
    assert context_of(run_hook("session-start", {"session_id": "nobody", "source": "compact"})) == SESSION_RULES


def test_subagent_start(fake_herdr):
    ctx = context_of(run_hook("subagent-start", {"agent_id": "a1", "agent_type": "Explore"}), "SubagentStart")
    assert "ask_operator" in ctx and "asked_by" in ctx


def test_broken_db_never_fails(tmp_path, fake_herdr):
    blocker = tmp_path / "file"
    blocker.write_text("")
    proc = run_hook("session-start", {"session_id": "s", "source": "compact"},
                    HERDR_INBOX_DB=str(blocker / "sub" / "inbox.db"))
    assert (proc.returncode, proc.stdout) == (0, "")


def test_garbage_input_never_fails(fake_herdr):
    proc = run_hook("session-start", "{not json")
    assert (proc.returncode, proc.stdout) == (0, "")


def test_plugin_hook_and_mcp_config_point_at_the_cli():
    hooks = json.loads((ROOT / "hooks/hooks.json").read_text())["hooks"]
    start = hooks["SessionStart"][0]
    assert set(start["matcher"].split("|")) == {"startup", "resume", "clear", "compact", "fork"}
    assert start["hooks"][0]["args"][-3:] == ["inbox", "hook", "session-start"]
    assert hooks["SubagentStart"][0]["hooks"][0]["args"][-3:] == ["inbox", "hook", "subagent-start"]
    server = json.loads((ROOT / ".claude-plugin/plugin.json").read_text())["mcpServers"]["herdr-inbox"]
    assert server["args"][-2:] == ["inbox", "mcp"] and "${CLAUDE_PLUGIN_ROOT}" in server["args"]
    assert (ROOT / "skills/operator-questions/SKILL.md").exists()
