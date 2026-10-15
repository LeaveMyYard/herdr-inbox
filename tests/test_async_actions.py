"""Nothing the operator or an agent does waits for herdr, tab labels or delivery.

Operator report: "sometimes sending herdr-inbox messages blocks all inputs and everything
just lags". Measured on an isolated herdr (PR description): the popup stayed open until the
send finished (the 15 s transcript check when an answer does not land), and an
ask_operator call waited 4 s for the tab-label lock held by another process."""

import os
import subprocess
import sys
import threading
import time

import pytest

from herdr_inbox import herdr, service, tui
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")


@pytest.fixture
def background(monkeypatch):
    monkeypatch.setattr(service, "_bg_queue", None)
    service.enable_background()
    yield
    service.drain(5)
    monkeypatch.setattr(service, "_bg_queue", None)


async def test_popup_records_hands_delivery_off_and_closes_at_once(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(tui, "DETACHED_SEND", True)
    spawned = []
    monkeypatch.setattr(tui, "spawn_delivery", lambda qids, force=False: spawned.append((qids, force)))
    qid = service.ask(store, A, "Merge?", options=["yes", "no"])["id"]
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        t = time.monotonic()
        await pilot.press("enter")
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert not app.is_running and time.monotonic() - t < 1.0
    assert store.get(qid).status == "answered" and store.get(qid).answer == "yes"
    assert not fake_herdr.calls_of("agent", "prompt")  # nothing typed by the popup itself
    assert spawned == [([qid], False)]


async def test_send_now_hands_off_with_force(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(tui, "DETACHED_SEND", True)
    spawned = []
    monkeypatch.setattr(tui, "spawn_delivery", lambda qids, force=False: spawned.append((qids, force)))
    qid = service.ask(store, A, "Late?", options=["yes"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "Other?")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.answer(store, qid, "yes")  # pending: agent busy
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down", "enter")
        await pilot.pause()
        await pilot.press("1")  # Send now
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert spawned == [([qid], True)]


def test_cli_deliver_delivers_recorded_answers(store, fake_herdr):
    from herdr_inbox import cli

    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    service.answer(store, qid, "yes", deliver=False)
    assert not fake_herdr.calls_of("agent", "prompt")
    assert cli.main(["deliver", str(qid)]) == 0
    assert store.get(qid).delivered_at and fake_herdr.calls_of("agent", "prompt")


def test_ask_does_not_wait_for_the_tab_label_lock(store, fake_herdr, background, monkeypatch, tmp_path):
    """Measured before: 4065 ms with the lock held by another process."""
    lock = str(store.path) + ".markers.lock"
    holder = subprocess.Popen([sys.executable, "-c", (
        "import fcntl,sys,time; f=open(sys.argv[1],'a+'); fcntl.flock(f, fcntl.LOCK_EX);"
        " print('x', flush=True); time.sleep(3)"), lock], stdout=subprocess.PIPE, text=True)
    holder.stdout.readline()
    try:
        t = time.monotonic()
        service.ask(store, A, "Quick?", options=["a"])
        assert time.monotonic() - t < 0.5
    finally:
        holder.wait()


def test_ask_notification_runs_in_the_background(store, fake_herdr, background, monkeypatch):
    threads = []
    monkeypatch.setattr(herdr, "notify", lambda *a: threads.append(threading.current_thread().name))
    service.ask(store, A, "Toast?")
    service.drain(5)
    assert threads == ["inbox-background"]


async def test_popup_never_calls_herdr_for_tab_labels_on_the_ui_thread(store, fake_herdr, background, monkeypatch):
    monkeypatch.setattr(tui, "LIVE_TAB_LABELS", True)
    calls = []
    monkeypatch.setattr(herdr, "tab_label", lambda tab: calls.append(threading.current_thread().name) or "Live")
    service.ask(store, A, "Where?")
    q = store.get(1)
    assert tui.tab_name(q, store) == "tab-A"  # the stored label at once
    service.drain(5)
    assert tui.tab_name(q, store) == "Live" and calls == ["inbox-background"]


async def test_go_to_agent_focuses_off_the_ui_thread(store, fake_herdr, monkeypatch):
    threads = []
    real = herdr.focus_agent
    monkeypatch.setattr(herdr, "focus_agent", lambda p: threads.append(threading.current_thread().name) or real(p))
    service.ask(store, A, "q1?")
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "q2?")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("g")
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert not app.is_running
    assert threads and threads[0] != threading.main_thread().name


def test_spawn_delivery_runs_inbox_deliver_detached(fake_herdr, monkeypatch):
    seen = []
    monkeypatch.setattr(tui.subprocess, "Popen", lambda args, **kw: seen.append((args, kw)))
    monkeypatch.setenv("UV_PROJECT_ENVIRONMENT", "/x/venv-0.6.2")
    tui.spawn_delivery([7, 8], force=True)
    args, kw = seen[0]
    assert args[1:] == ["-m", "herdr_inbox.cli", "deliver", "7", "8", "--force"]
    assert kw["start_new_session"] and "UV_PROJECT_ENVIRONMENT" not in kw["env"]


def test_inbox_trace_logs_steps_only_when_set(store, fake_herdr, monkeypatch, tmp_path):
    import json

    log = tmp_path / "trace.jsonl"
    service.ask(store, A, "No trace?")
    assert not log.exists()
    monkeypatch.setenv("INBOX_TRACE", str(log))
    service.ask(store, A, "Traced?")
    steps = [json.loads(line) for line in log.read_text().splitlines()]
    assert any(s["step"] == "herdr notification show" and "ms" in s and s["thread"] for s in steps)
