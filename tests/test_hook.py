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
    c = Caller(pane_id="w1:p1", tab_id="w1:t1", tab_label="t", session_id="aaaaaaaa-0000-4000-8000-00000000000a", cwd="/r")
    service.ask(store, c, "Which region?")
    answered = service.ask(store, c, "Squash or merge?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")  # delivered answers count as seen
    service.answer(store, answered, "squash")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    service.ask(store, Caller("w1:p2", None, None, "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Other session question?")


def test_rules_are_short():
    assert len(SESSION_RULES.split()) < 250


def test_startup_injects_rules(fake_herdr):
    ctx = context_of(run_hook("session-start", {"session_id": "aaaaaaaa-0000-4000-8000-00000000000a", "source": "startup"}))
    assert ctx == SESSION_RULES
    for phrase in ("ask_operator", "Ask once", "`[inbox]` messages naming the Q ids", "list_my_questions",
                   "destructive", "subagent", "herdr-inbox:operator-questions"):
        assert phrase in ctx


def test_startup_does_not_list_questions(store, fake_herdr, monkeypatch):
    seed(store, monkeypatch)
    ctx = context_of(run_hook("session-start", {"session_id": "aaaaaaaa-0000-4000-8000-00000000000a", "source": "startup"}))
    assert "Q1" not in ctx


def test_compact_lists_this_sessions_pending_questions(store, fake_herdr, monkeypatch):
    seed(store, monkeypatch)
    ctx = context_of(run_hook("session-start", {"session_id": "aaaaaaaa-0000-4000-8000-00000000000a", "source": "compact"}))
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
    # since 0.5.3 (review of #11): an unusable DB costs the pending list, never the rules
    assert proc.returncode == 0
    context = json.loads(proc.stdout)["hookSpecificOutput"]["additionalContext"]
    assert context.startswith("<herdr-inbox>") and "already with the operator" not in context


def test_garbage_input_never_fails(fake_herdr):
    proc = run_hook("session-start", "{not json")
    assert (proc.returncode, proc.stdout) == (0, "")


def test_plugin_hook_and_mcp_config_point_at_the_cli():
    hooks = json.loads((ROOT / "hooks/hooks.json").read_text())["hooks"]
    start = hooks["SessionStart"][0]
    assert set(start["matcher"].split("|")) == {"startup", "resume", "clear", "compact", "fork"}
    # stdlib runner behind an always-exit-0 sh wrapper (tests/test_hook_launch.py)
    assert start["hooks"][0]["args"][-2:] == ["${CLAUDE_PLUGIN_ROOT}/hooks/run.py", "session-start"]
    assert hooks["SubagentStart"][0]["hooks"][0]["args"][-2:] == ["${CLAUDE_PLUGIN_ROOT}/hooks/run.py", "subagent-start"]
    server = json.loads((ROOT / ".claude-plugin/plugin.json").read_text())["mcpServers"]["herdr-inbox"]
    assert server["args"][-2:] == ["inbox", "mcp"] and "${CLAUDE_PLUGIN_ROOT}" in server["args"]
    assert (ROOT / "skills/operator-questions/SKILL.md").exists()


def _raising(codes):
    """A ``work`` for hook._bounded: raises an OperationalError with each extended code in
    ``codes`` in turn, then returns how many times it ran."""
    import sqlite3

    calls = []

    def work(db):
        calls.append(1)
        if len(calls) <= len(codes):
            exc = sqlite3.OperationalError("database is locked")
            exc.sqlite_errorcode = codes[len(calls) - 1]
            raise exc
        return len(calls)

    return work


def test_bounded_retries_extended_busy_codes():
    """Review #59 M: BUSY_RECOVERY (261, a reader's usual BUSY in WAL mode), BUSY_SNAPSHOT
    (517) and LOCKED_SHAREDCACHE (262) are busy too: retried, not given up on."""
    import sqlite3

    from herdr_inbox import hook

    for code in (sqlite3.SQLITE_BUSY, 261, 517, sqlite3.SQLITE_LOCKED, 262):
        assert hook._bounded(lambda: sqlite3.connect(":memory:"), _raising([code]), timeout=5) == 2, code


def test_bounded_gives_up_on_other_errors_and_at_the_deadline():
    import sqlite3
    import time

    import pytest

    from herdr_inbox import hook

    work = _raising([1])  # SQLITE_ERROR: not busy, raised at once
    with pytest.raises(sqlite3.OperationalError):
        hook._bounded(lambda: sqlite3.connect(":memory:"), work, timeout=5)
    started = time.monotonic()
    with pytest.raises(sqlite3.OperationalError):  # busy every time: gives up at the deadline
        hook._bounded(lambda: sqlite3.connect(":memory:"), _raising([5] * 10_000), timeout=0.1)
    assert time.monotonic() - started < 5


LOOKUP_BUDGET = 0.5  # what a locked DB may cost a delivery-hook lookup (hook.PENDING_TIMEOUT 0.2 s + slack)


def _locked(mode: str):
    """A connection holding the inbox DB in rollback-journal mode: BEGIN EXCLUSIVE blocks
    readers and writers, BEGIN IMMEDIATE only writers."""
    import os
    import sqlite3

    from herdr_inbox.store import Store

    Store(migrate=True).close()
    lock = sqlite3.connect(os.environ["HERDR_INBOX_DB"], isolation_level=None)
    lock.execute("PRAGMA journal_mode=DELETE")
    lock.execute(f"BEGIN {mode}")
    return lock


def _timed(fn, *args):
    import time

    started = time.monotonic()
    result = fn(*args)
    return result, time.monotonic() - started


def test_touch_gives_up_quickly_on_a_locked_db(fake_herdr):
    """Final pass #59 L: the delivery hook's liveness stamp is bounded by wall clock too,
    both its read-only look (EXCLUSIVE: readers blocked) and its write (IMMEDIATE: the read
    finds no stamp, the write is blocked)."""
    import sqlite3

    from herdr_inbox import hook

    for mode in ("EXCLUSIVE", "IMMEDIATE"):
        lock = _locked(mode)
        try:
            result, took = _timed(hook._touch, "s")
        finally:
            lock.execute("ROLLBACK")
            lock.close()
        assert result is None and took < LOOKUP_BUDGET, (mode, took)
    ro = sqlite3.connect(f"file:{__import__('os').environ['HERDR_INBOX_DB']}?mode=ro", uri=True)
    try:  # nothing was stamped while locked
        assert ro.execute("SELECT hook_seen_at FROM sessions WHERE session_id = 's'").fetchone() is None
    finally:
        ro.close()


def test_has_candidates_gives_up_quickly_on_a_locked_db(store, fake_herdr, monkeypatch):
    """Final pass #59 L: the delivery hook's candidate check is bounded by wall clock: on a
    locked DB it says "nothing" within the budget (the next firing looks again)."""
    from herdr_inbox import hook

    seed(store, monkeypatch)  # Q2 is answered and not yet seen: a candidate when unlocked
    store.close()
    assert hook._has_candidates("aaaaaaaa-0000-4000-8000-00000000000a")
    lock = _locked("EXCLUSIVE")
    try:
        result, took = _timed(hook._has_candidates, "aaaaaaaa-0000-4000-8000-00000000000a")
    finally:
        lock.execute("ROLLBACK")
        lock.close()
    assert result is False and took < LOOKUP_BUDGET, took


def test_bounded_closes_every_connection_it_opens():
    """Each retry opens a fresh connection; none is left open, whether the work succeeds,
    keeps failing busy until the deadline, or fails otherwise."""
    import sqlite3

    import pytest

    from herdr_inbox import hook

    def run(work, timeout=5):
        opened = []

        def connect():
            opened.append(sqlite3.connect(":memory:"))
            return opened[-1]

        try:
            hook._bounded(connect, work, timeout=timeout)
        except sqlite3.OperationalError:
            pass
        for db in opened:
            with pytest.raises(sqlite3.ProgrammingError):  # closed
                db.execute("SELECT 1")
        return len(opened)

    assert run(_raising([5, 261])) == 3
    assert run(_raising([5] * 10_000), timeout=0.05) > 1
    assert run(_raising([1])) == 1
