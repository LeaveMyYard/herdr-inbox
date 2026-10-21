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
from datetime import timedelta
from pathlib import Path

import pytest

from herdr_inbox import service, worker
from herdr_inbox.service import Caller
from herdr_inbox.store import Store, now

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")


@pytest.fixture(autouse=True)
def quick_verify(monkeypatch):
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.3)


@pytest.fixture
def live_session(fake_herdr):
    path = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-fake-project" / "aaaaaaaa-0000-4000-8000-00000000000a.jsonl"
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
    monkeypatch.setenv("FAKE_HERDR_SCREEN", SCREEN_PANEL_FOCUSED)
    qid = pending_answer(store, monkeypatch, status="working")
    worker.tick(store)
    assert prompts(fake_herdr) == []
    assert "panel" in store.get(qid).delivery_error
    monkeypatch.setenv("FAKE_HERDR_SCREEN", SCREEN_NORMAL_WITH_PHRASE_IN_BODY)
    worker.tick(store)
    assert store.get(qid).verified_at


def test_an_old_servers_unverified_ack_is_requeued_and_delivered(store, fake_herdr, monkeypatch, live_session):
    """A pre-0.5.1 server types and marks the answer seen in one go; it never landed. The
    worker's reconcile un-sees it AND puts it back for delivery (the live 11-minute gap)."""
    qid = service.ask(store, A, "Merge it?", options=["yes"])["id"]
    store.set_answer(qid, "yes")
    from datetime import timedelta

    t = now(-timedelta(seconds=60))  # settled: older than the verify window
    store.db.execute("UPDATE questions SET delivered_at = ?, acknowledged_at = ? WHERE id = ?", (t, t, qid))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    worker.tick(store)
    q = store.get(qid)
    assert q.verified_at and len(prompts(fake_herdr)) == 1


def test_old_requeue_window_does_not_retype_old_answers(store, fake_herdr, monkeypatch, live_session):
    qid = service.ask(store, A, "Old?", options=["yes"])["id"]
    store.set_answer(qid, "yes")
    # inside the one-off 7-day sweep (RESET_WINDOW), outside the 1 h requeue and 24 h signature windows;
    # relative to now: a fixed date drifts out of the window (#88)
    old = now(-timedelta(days=2))
    store.db.execute("UPDATE questions SET delivered_at = ?, acknowledged_at = ?, answered_at = ? WHERE id = ?",
                     (old, old, old, qid))
    from herdr_inbox.store import RESET_KEY

    store.db.execute("DELETE FROM meta WHERE key = ?", (RESET_KEY,))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    worker.tick(store)
    assert prompts(fake_herdr) == [] and store.get(qid).acknowledged_at is None  # un-seen only


def test_stuck_answers_raise_one_notification(store, fake_herdr, monkeypatch, live_session):
    monkeypatch.setattr(service, "BUSY_GRACE", 1e9)  # busy all along: pending, never typed
    a = pending_answer(store, monkeypatch, status="working")
    b = pending_answer(store, monkeypatch, status="working")
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


# -- review of #15 ---------------------------------------------------------------------------


def test_every_answer_or_send_ensures_the_worker(store, fake_herdr, monkeypatch, live_session):
    """M1: a worker that idled out must come back as soon as the operator answers."""
    started = []
    monkeypatch.setattr(worker, "ensure", lambda *a: started.append(1) or True)
    qid = pending_answer(store, monkeypatch)  # service.answer
    assert started
    started.clear()
    service.deliver(store, store.get(qid), force=True)  # the popup's Send now / Retry
    assert started
    started.clear()
    service.retry_deliveries(store, "w1:p1")  # the worker's own retries do not re-spawn
    assert not started


def test_a_worker_that_idles_out_with_new_work_starts_a_successor(fake_herdr, monkeypatch):
    """M1 race: an answer committed while the worker decides to exit is not orphaned."""
    monkeypatch.setattr(worker, "IDLE_EXIT", 0.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    calls = {"n": 0}

    def tick(store):
        calls["n"] += 1
        return 0  # nothing pending at decision time

    monkeypatch.setattr(worker, "tick", tick)
    monkeypatch.setattr(worker, "_pending_now", lambda: 1)  # ...but an answer just arrived
    started = []
    monkeypatch.setattr(worker, "ensure", lambda *a: started.append(1) or True)
    assert worker.run() == 0
    assert started == [1]


SCREEN_NORMAL_WITH_PHRASE_IN_BODY = """\
● The panel footer reads "↑/↓ to select" when it has focus.
✻ Waiting for 3 background agents to finish
──────────────────────────────────────────────
❯
──────────────────────────────────────────────
  ⏵⏵ auto mode on (shift+tab to cycle) · ← for agents · ↓ to manage
  ● main
  ◯ general-purpose Compute (1)"""

SCREEN_PANEL_FOCUSED = """\
✻ Waiting for 3 background agents to finish
──────────────────────────────────────────────
❯
──────────────────────────────────────────────
  ↑/↓ to select · ctrl+x ctrl+k to stop all agents
❯ ● main
  ◯ general-purpose Compute (1)"""


def test_panel_detection_reads_only_the_footer(fake_herdr, monkeypatch):
    """M2: the phrase in ordinary output (this repo's own docs!) is not a focused panel."""
    from herdr_inbox import herdr

    monkeypatch.setenv("FAKE_HERDR_SCREEN", SCREEN_NORMAL_WITH_PHRASE_IN_BODY)
    assert not herdr.panel_focused("w1:p1")
    monkeypatch.setenv("FAKE_HERDR_SCREEN", SCREEN_PANEL_FOCUSED)
    assert herdr.panel_focused("w1:p1")
    monkeypatch.setenv("FAKE_HERDR_SCREEN", "↑/↓ to select")  # no input box: not Claude's footer
    assert not herdr.panel_focused("w1:p1")


def test_send_now_overrides_the_panel_wait(store, fake_herdr, monkeypatch, live_session):
    monkeypatch.setenv("FAKE_HERDR_SCREEN", SCREEN_PANEL_FOCUSED)
    qid = pending_answer(store, monkeypatch, status="idle")
    assert prompts(fake_herdr) == [] and "panel" in store.get(qid).delivery_error
    service.deliver(store, store.get(qid), force=True)
    assert len(prompts(fake_herdr)) == 1 and store.get(qid).verified_at  # verification is the net


def test_unverifiable_sends_are_not_reported_as_not_delivered(store, fake_herdr, monkeypatch):
    """Low: no transcript -> sent, unseen by design; it most likely landed."""
    from herdr_inbox import transcript

    monkeypatch.setattr(transcript, "find", lambda *a, **k: None)
    qid = service.ask(store, A, "No transcript?", options=["yes"])["id"]
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    service.answer(store, qid, "yes")  # no live_session: nothing to verify against
    store.db.execute("UPDATE questions SET answered_at = ? WHERE id = ?", ("2026-10-15T00:00:00+00:00", qid))
    q = store.get(qid)
    assert q.delivered_at and q.verified_at is None and q.acknowledged_at is None  # the no-transcript path
    worker.tick(store)
    assert not [c for c in fake_herdr.calls_of("notification", "show") if "not delivered" in c[2]]


def test_an_agent_at_a_prompt_gets_a_waiting_notification(store, fake_herdr, monkeypatch, live_session):
    """Low: blocked answers are not lost; point the operator at the pane, not check_answers."""
    qid = pending_answer(store, monkeypatch, status="blocked")
    store.db.execute("UPDATE questions SET answered_at = ? WHERE id = ?", ("2026-10-15T00:00:00+00:00", qid))
    worker.tick(store)
    shows = [c for c in fake_herdr.calls_of("notification", "show") if c[2].startswith("waiting:")]
    assert len(shows) == 1 and "is at a prompt" in shows[0][2] and "check_answers" not in shows[0][4]


def test_the_worker_hands_over_when_the_disk_version_differs_from_its_own(fake_herdr, monkeypatch):
    """Low (L1): compare the RUNNING code's version with the version on disk."""
    monkeypatch.setattr(worker, "IDLE_EXIT", 3600.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    monkeypatch.setattr(worker, "running_version", lambda: "0.6.1")
    disk = iter(["0.6.1", "0.6.2"])
    monkeypatch.setattr(worker, "installed_version", lambda: next(disk, "0.6.2"))
    monkeypatch.setattr(worker, "tick", lambda store: 1)  # always something pending
    successors = []
    monkeypatch.setattr(worker, "start_successor", lambda: successors.append(1) or True)
    assert worker.run() == 0
    assert successors == [1]


def test_no_handover_loop_when_no_successor_can_start(fake_herdr, monkeypatch):
    monkeypatch.setattr(worker, "IDLE_EXIT", 3600.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    monkeypatch.setattr(worker, "running_version", lambda: "0.6.1")
    monkeypatch.setattr(worker, "installed_version", lambda: "0.6.2")
    ticks = {"n": 0}

    def tick(store):
        ticks["n"] += 1
        if ticks["n"] >= 4:
            raise KeyboardInterrupt  # stop the test loop: the worker kept running
        return 1

    monkeypatch.setattr(worker, "tick", tick)
    monkeypatch.setattr(worker, "start_successor", lambda: False)  # e.g. no uv
    with pytest.raises(KeyboardInterrupt):
        worker.run()
    assert ticks["n"] == 4


# -- final pass of #15 -----------------------------------------------------------------------


def _signature(store, qid, seconds_ago):
    from datetime import timedelta

    t = now(-timedelta(seconds=seconds_ago))
    store.db.execute("UPDATE questions SET delivered_at = ?, acknowledged_at = ? WHERE id = ?", (t, t, qid))


def test_h1_a_just_typed_old_server_delivery_is_not_requeued_before_it_settles(store, fake_herdr, monkeypatch, live_session):
    """H1 race: the old server wrote delivered=acked a moment before Claude Code wrote the
    transcript record; requeuing then would type it twice."""
    qid = service.ask(store, A, "Late?", options=["yes"])["id"]
    store.set_answer(qid, "yes")
    _signature(store, qid, 0)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    worker.tick(store)  # not settled: left alone
    assert prompts(fake_herdr) == [] and store.get(qid).acknowledged_at
    with open(live_session, "a") as fh:  # ...the original lands
        fh.write(json.dumps({"type": "user", "message": {"content": f"[inbox] Q{qid} answered — Late?: yes"}}) + "\n")
    _signature(store, qid, 60)
    worker.tick(store)
    assert prompts(fake_herdr) == [] and store.get(qid).verified_at


def test_h1_no_transcript_means_un_see_only_never_retype(store, fake_herdr, monkeypatch, live_session):
    from herdr_inbox import transcript

    qid = service.ask(store, A, "Blind?", options=["yes"])["id"]
    store.set_answer(qid, "yes")
    _signature(store, qid, 60)
    monkeypatch.setattr(transcript, "find", lambda *a, **k: None)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    worker.tick(store)
    q = store.get(qid)
    assert prompts(fake_herdr) == [] and q.acknowledged_at is None and q.delivered_at


def test_h1_a_retry_checks_the_transcript_before_typing_again(store, fake_herdr, monkeypatch, live_session):
    """A NOT_VERIFIED answer whose record landed after the verify window: verified, not retyped."""
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    qid = pending_answer(store, monkeypatch, status="idle")
    assert store.get(qid).delivery_error == service.NOT_VERIFIED and len(prompts(fake_herdr)) == 1
    with open(live_session, "a") as fh:  # the late record
        # the real message (0.6.3: the check matches the full text, not the id alone)
        fh.write(json.dumps({"type": "user", "message": {"content": service.delivery_text([store.get(qid)])}}) + "\n")
    worker.tick(store)
    assert len(prompts(fake_herdr)) == 1 and store.get(qid).verified_at


def test_m1_the_lock_is_per_herdr_server(fake_herdr, monkeypatch, tmp_path):
    monkeypatch.setenv("HERDR_SOCKET_PATH", str(tmp_path / "a.sock"))
    a = worker.hold_lock()
    monkeypatch.setenv("HERDR_SOCKET_PATH", str(tmp_path / "b.sock"))
    b = worker.hold_lock()
    try:
        assert a is not None and b is not None  # one worker per server
    finally:
        a.close()
        b.close()


def test_m1_questions_record_their_herdr_server(store, fake_herdr, monkeypatch, tmp_path):
    monkeypatch.setenv("HERDR_SOCKET_PATH", str(tmp_path / "a.sock"))
    qid = service.ask(store, A, "Where?")["id"]
    assert store.server_of(qid) == str(tmp_path / "a.sock")


def test_m1_a_worker_leaves_other_servers_rows_alone(store, fake_herdr, monkeypatch, tmp_path, live_session):
    monkeypatch.setenv("HERDR_SOCKET_PATH", str(tmp_path / "other.sock"))
    qid = pending_answer(store, monkeypatch)
    attempts = store.get(qid).delivery_attempts
    monkeypatch.setenv("HERDR_SOCKET_PATH", str(tmp_path / "mine.sock"))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    fake_herdr.reset()
    for _ in range(3):
        worker.tick(store)
    assert prompts(fake_herdr) == [] and store.get(qid).delivery_attempts == attempts
    assert not fake_herdr.calls_of("agent", "get")  # not even looked up on the wrong server


def test_m1_legacy_rows_need_a_matching_session(store, fake_herdr, monkeypatch, live_session):
    """Rows asked before 0.6.1 carry no server: only a live session match may type them."""
    qid = pending_answer(store, monkeypatch)
    store.db.execute("UPDATE question_ext SET herdr_server = NULL WHERE id = ?", (qid,))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    monkeypatch.setenv("FAKE_HERDR_SESSION", "someone-else")  # that pane id, another conversation
    worker.tick(store)
    q = store.get(qid)
    assert prompts(fake_herdr) == [] and q.delivery_attempts == 0  # no caps burnt
    monkeypatch.setenv("FAKE_HERDR_SESSION", "aaaaaaaa-0000-4000-8000-00000000000a")
    store.db.execute("UPDATE question_ext SET legacy_checked_at = NULL WHERE id = ?", (qid,))  # backoff over
    worker.tick(store)
    assert store.get(qid).verified_at
    qid2 = pending_answer(store, monkeypatch)
    store.db.execute("UPDATE question_ext SET herdr_server = NULL WHERE id = ?", (qid2,))
    store.db.execute("UPDATE questions SET session_id = NULL WHERE id = ?", (qid2,))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    n = len(prompts(fake_herdr))
    worker.tick(store)
    assert len(prompts(fake_herdr)) == n  # never a pane-id-only match


def test_m2_spawns_do_not_inherit_the_mcp_venv(fake_herdr, monkeypatch):
    monkeypatch.delenv("HERDR_INBOX_NO_WORKER")
    monkeypatch.setenv("UV_PROJECT_ENVIRONMENT", "/data/venv-0.6.1")
    monkeypatch.setenv("VIRTUAL_ENV", "/data/venv-0.6.1")
    spawned = []
    from herdr_inbox import herdr

    monkeypatch.setattr(herdr, "current_plugin_root", herdr._own_checkout)  # asks herdr through Popen
    monkeypatch.setattr(worker.subprocess, "Popen", lambda *a, **k: spawned.append(k))
    worker.ensure()
    assert worker.start_successor() is True
    for kwargs in spawned:
        env = kwargs["env"]
        assert "UV_PROJECT_ENVIRONMENT" not in env and "VIRTUAL_ENV" not in env


def test_l3_the_focus_check_is_repeated_right_before_typing(store, fake_herdr, monkeypatch, live_session):
    from herdr_inbox import herdr

    qid = pending_answer(store, monkeypatch)
    real = herdr.live_agent
    seen = {"n": 0}

    def live_agent(pane):
        agent = real(pane)
        seen["n"] += 1
        if agent and seen["n"] > 1:
            agent = {**agent, "focused": True}  # the operator stepped into the pane meanwhile
        return agent

    monkeypatch.setattr(herdr, "live_agent", live_agent)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    service.retry_deliveries(store, "w1:p1")
    assert prompts(fake_herdr) == [] and "operator" in store.get(qid).delivery_error


# -- follow-up (review of #15, Low): pending counts only what this worker serves ------------


def test_other_servers_rows_do_not_keep_a_worker_alive(store, fake_herdr, monkeypatch, tmp_path, live_session):
    monkeypatch.setenv("HERDR_SOCKET_PATH", str(tmp_path / "other.sock"))
    pending_answer(store, monkeypatch)  # pending, but on another herdr server
    monkeypatch.setenv("HERDR_SOCKET_PATH", str(tmp_path / "mine.sock"))
    assert worker.tick(store) == 0
    assert worker._pending_now() == 0


def test_unmatched_legacy_rows_do_not_keep_a_worker_alive(store, fake_herdr, monkeypatch, live_session):
    qid = pending_answer(store, monkeypatch)
    store.db.execute("UPDATE question_ext SET herdr_server = NULL WHERE id = ?", (qid,))
    monkeypatch.setenv("FAKE_HERDR_SESSION", "someone-else")
    assert worker.tick(store) == 0


def test_own_pending_rows_still_count(store, fake_herdr, monkeypatch, live_session):
    pending_answer(store, monkeypatch)  # blocked agent: stays pending, on this server
    assert worker.tick(store) == 1 and worker._pending_now() == 1
