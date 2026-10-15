"""0.6.1: one herdr-side delivery worker retries every pending answer.

Live (0.6.0): Q49-Q52 were typed while the agent's background-agents panel had focus, never
landed, and nothing retried for 11 minutes: the retry loop lived in the asking session's own
MCP server, and that session still ran a pre-0.5.1 server. Now a single worker (lock-guarded,
started by herdr's [[startup]], the popup, the CLI or any 0.6.1 MCP server) owns retries,
verification and the busy/idle policy for ALL panes, whatever version the sessions run."""

import json
import os
import subprocess
import sys
import time
from pathlib import Path

import pytest

from herdr_inbox import service, worker
from herdr_inbox.service import Caller
from herdr_inbox.store import Store, now

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")


@pytest.fixture(autouse=True)
def quick_verify(monkeypatch):
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.3)


@pytest.fixture
def live_session(fake_herdr):
    path = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-fake-project" / "sess-A.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"type": "user", "message": {"content": "hello"}}) + "\n")
    return path


def prompts(fake):
    return [c[3] for c in fake.calls_of("agent", "prompt")]


_n = iter(range(1000))


def pending_answer(store, monkeypatch, caller=A, status="blocked"):
    qid = service.ask(store, caller, f"Merge it {next(_n)}?", options=["yes", "no"])["id"]
    monkeypatch.setenv("FAKE_HERDR_STATUS", status)
    service.answer(store, qid, "yes")
    return qid


def test_one_tick_delivers_pending_answers_for_every_pane(store, fake_herdr, monkeypatch, live_session):
    """Not only the worker's own pane: the old per-session loop only served HERDR_PANE_ID."""
    monkeypatch.delenv("HERDR_PANE_ID")
    a, b = pending_answer(store, monkeypatch, A), pending_answer(store, monkeypatch, B)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    worker.tick(store)
    assert store.get(a).verified_at and store.get(b).verified_at
    targets = [c[2] for c in fake_herdr.calls_of("agent", "prompt")]
    assert sorted(targets) == ["w1:p1", "w1:p2"]  # one message per pane


def test_a_busy_miss_waits_for_idle_and_shows_as_not_delivered(store, fake_herdr, monkeypatch, live_session):
    """Typed while busy, never seen (the agents panel ate it): no more typing while busy;
    the next idle/done gets it, and the popup says ⚠ at once."""
    from herdr_inbox import tui

    monkeypatch.setattr(service, "BUSY_GRACE", 0.0)
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    qid = pending_answer(store, monkeypatch, status="working")
    worker.tick(store)
    worker.tick(store)
    assert len(prompts(fake_herdr)) == 1  # one busy try, then wait for idle
    q = store.get(qid)
    assert tui.pending_text(q.delivery_error[len("pending: "):], 99).startswith("⚠ not delivered — agent busy")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "done")
    monkeypatch.setenv("FAKE_HERDR_LAND", "user")
    worker.tick(store)
    assert store.get(qid).verified_at and len(prompts(fake_herdr)) == 2


def test_a_focused_task_panel_is_not_typed_into(store, fake_herdr, monkeypatch, live_session):
    """Probe 2026-10-15: with Claude Code's background-agents panel focused (footer '↑/↓ to
    select'), typed text never reaches the conversation."""
    monkeypatch.setattr(service, "BUSY_GRACE", 0.0)
    monkeypatch.setenv("FAKE_HERDR_SCREEN", "❯\n ↑/↓ to select · ctrl+x ctrl+k to stop all agents\n ◯ main")
    qid = pending_answer(store, monkeypatch, status="working")
    worker.tick(store)
    assert prompts(fake_herdr) == []
    assert "panel" in store.get(qid).delivery_error
    monkeypatch.setenv("FAKE_HERDR_SCREEN", "❯\n ⏵⏵ auto mode on · ← for agents · ↓ to manage")
    worker.tick(store)
    assert store.get(qid).verified_at


def test_an_old_servers_unverified_ack_is_requeued_and_delivered(store, fake_herdr, monkeypatch, live_session):
    """A pre-0.5.1 server types and marks the answer seen in one go; it never landed. The
    worker's reconcile un-sees it AND puts it back for delivery (the live 11-minute gap)."""
    qid = service.ask(store, A, "Merge it?", options=["yes"])["id"]
    store.set_answer(qid, "yes")
    t = now()
    store.db.execute("UPDATE questions SET delivered_at = ?, acknowledged_at = ? WHERE id = ?", (t, t, qid))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    worker.tick(store)
    q = store.get(qid)
    assert q.verified_at and len(prompts(fake_herdr)) == 1


def test_old_requeue_window_does_not_retype_old_answers(store, fake_herdr, monkeypatch, live_session):
    qid = service.ask(store, A, "Old?", options=["yes"])["id"]
    store.set_answer(qid, "yes")
    old = "2026-10-14T00:00:00+00:00"
    store.db.execute("UPDATE questions SET delivered_at = ?, acknowledged_at = ?, answered_at = ? WHERE id = ?",
                     (old, old, old, qid))
    from herdr_inbox.store import RESET_KEY

    store.db.execute("DELETE FROM meta WHERE key = ?", (RESET_KEY,))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    worker.tick(store)
    assert prompts(fake_herdr) == [] and store.get(qid).acknowledged_at is None  # un-seen only


def test_stuck_answers_raise_one_notification(store, fake_herdr, monkeypatch, live_session):
    a = pending_answer(store, monkeypatch)
    b = pending_answer(store, monkeypatch)
    store.db.execute("UPDATE questions SET answered_at = ? WHERE id IN (?, ?)",
                     ("2026-10-15T00:00:00+00:00", a, b))
    worker.tick(store)
    worker.tick(store)
    shows = [c for c in fake_herdr.calls_of("notification", "show") if "not delivered" in c[2]]
    assert len(shows) == 1
    assert f"Q{a}–Q{b} not delivered to my-tab" in shows[0][2] and "check_answers" in shows[0][4]


def test_the_worker_is_a_singleton(fake_herdr, tmp_path):
    held = worker.hold_lock()
    assert held is not None and worker.is_running()
    assert worker.hold_lock() is None  # a second worker does not start
    held.close()
    assert not worker.is_running()


def test_ensure_spawns_only_when_not_running(fake_herdr, monkeypatch):
    monkeypatch.delenv("HERDR_INBOX_NO_WORKER")
    spawned = []
    monkeypatch.setattr(worker.subprocess, "Popen", lambda *a, **k: spawned.append((a, k)))
    assert worker.ensure() is True and len(spawned) == 1
    argv, kwargs = spawned[0][0][0], spawned[0][1]
    assert argv[-1] == "worker" and kwargs["start_new_session"] is True
    held = worker.hold_lock()
    try:
        assert worker.ensure() is False and len(spawned) == 1
    finally:
        held.close()


def test_worker_process_runs_and_exits_when_idle(fake_herdr, monkeypatch, tmp_path):
    env = {**os.environ, "HERDR_INBOX_WORKER_IDLE_EXIT": "0.5", "HERDR_INBOX_WORKER_TICK": "0.1"}
    proc = subprocess.run([sys.executable, "-m", "herdr_inbox.cli", "worker"], env=env, timeout=30,
                          capture_output=True, text=True)
    assert proc.returncode == 0, proc.stderr


def test_mcp_servers_no_longer_run_their_own_retry_loop():
    from herdr_inbox import server

    assert not hasattr(server, "start_retry_loop")


def test_herdr_startup_ensures_the_worker():
    import tomllib

    m = tomllib.loads((Path(__file__).resolve().parents[1] / "herdr-plugin.toml").read_text())
    assert any(s["command"][-2:] == ["worker", "--ensure"] for s in m["startup"])
