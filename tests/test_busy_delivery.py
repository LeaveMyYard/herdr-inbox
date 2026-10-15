"""0.5.4: an agent that stays `working` for a long time still gets its answer.

Live (0.5.3): Q45's pane showed "Waiting for 1 dynamic workflow to finish", which herdr
reports as `working` for as long as the workflow runs (hours), yet Claude Code accepts
typed input then. Idle-only typing held the answer as "pending delivery" indefinitely.
Now: idle/done → type now; working → wait BUSY_GRACE, then type anyway and let the
transcript check (2-attempt cap) be the safety net; blocked (a prompt or dialog swallows
input) → keep waiting. The popup explains the wait and offers "Send now"."""

import json
import os
from pathlib import Path

import pytest

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import FailedView, GroupListView, InboxApp

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r")


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


def age_pending(store, qid, seconds):
    from herdr_inbox.store import now

    store.db.execute("UPDATE question_ext SET pending_since = ? WHERE id = ?", (now(-__import__("datetime").timedelta(seconds=seconds)), qid))


def answered_while(store, monkeypatch, status, text="Merge it?"):
    qid = service.ask(store, A, text, options=["yes", "no"])["id"]
    monkeypatch.setenv("FAKE_HERDR_STATUS", status)
    service.answer(store, qid, "yes")
    return qid


def test_working_agent_waits_the_grace_then_gets_it_anyway(store, fake_herdr, monkeypatch, live_session):
    monkeypatch.setenv("FAKE_HERDR_LAND", "queue")  # queued mid-turn, as the probe recorded
    qid = answered_while(store, monkeypatch, "working")
    q = store.get(qid)
    assert prompts(fake_herdr) == [] and q.delivery_error.startswith("pending: busy")
    service.retry_deliveries(store, "w1:p1")  # still within the grace: not yet
    assert prompts(fake_herdr) == []
    age_pending(store, qid, service.BUSY_GRACE + 1)
    service.retry_deliveries(store, "w1:p1")  # still working, grace over: typed, verified
    q = store.get(qid)
    assert len(prompts(fake_herdr)) == 1 and q.verified_at and q.acknowledged_at
    assert q.delivery_error is None


def test_busy_misses_have_their_own_cap_then_wait_for_idle(store, fake_herdr, monkeypatch, live_session):
    """Review of #13, M1: typing into a `working` agent that swallows it is tried
    MAX_BUSY_ATTEMPTS times (1 since 0.6.1), then the answer waits for idle -- never parked."""
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    qid = answered_while(store, monkeypatch, "working")
    for _ in range(8):
        age_pending(store, qid, service.BUSY_GRACE + 1)
        service.retry_deliveries(store, "w1:p1")
    q = store.get(qid)
    assert len(prompts(fake_herdr)) == service.MAX_BUSY_ATTEMPTS == 1
    assert q.delivery_error.startswith("pending: busy") and q.delivery_attempts == 0
    assert "when it is idle" in __import__("herdr_inbox.tui").tui.pending_text(q.delivery_error[len("pending: "):], 99)


def test_reviewer_sequence_working_misses_then_idle_delivers(store, fake_herdr, monkeypatch, live_session):
    """The reviewer's repro: working + swallowed, then idle: it must end delivered."""
    monkeypatch.setattr(service, "BUSY_GRACE", 0.0)
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    qid = answered_while(store, monkeypatch, "working")
    for _ in range(5):
        service.retry_deliveries(store, "w1:p1")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    monkeypatch.setenv("FAKE_HERDR_LAND", "user")
    service.retry_deliveries(store, "w1:p1")
    q = store.get(qid)
    assert q.verified_at and q.acknowledged_at and q.delivery_error is None


def test_idle_misses_keep_the_two_attempt_cap(store, fake_herdr, monkeypatch, live_session):
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    qid = answered_while(store, monkeypatch, "idle")
    for _ in range(5):
        service.retry_deliveries(store, "w1:p1")
    assert len(prompts(fake_herdr)) == service.MAX_UNVERIFIED_ATTEMPTS
    assert store.get(qid).delivery_error == service.NOT_VERIFIED


def test_time_at_a_dialog_does_not_use_up_the_busy_grace(store, fake_herdr, monkeypatch, live_session):
    """Review of #13, Low: the grace counts from the first `working` moment."""
    qid = answered_while(store, monkeypatch, "blocked")
    assert store.get_pending_since(qid) is None
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.retry_deliveries(store, "w1:p1")
    assert store.get_pending_since(qid) and prompts(fake_herdr) == []


def test_blocked_agent_keeps_waiting(store, fake_herdr, monkeypatch, live_session):
    qid = answered_while(store, monkeypatch, "blocked")
    age_pending(store, qid, 3600)
    service.retry_deliveries(store, "w1:p1")
    service.deliver(store, store.get(qid), force=True)  # not even "Send now": a dialog eats it
    assert prompts(fake_herdr) == []
    assert store.get(qid).delivery_error.startswith("pending: blocked")


def test_send_now_skips_the_grace(store, fake_herdr, monkeypatch, live_session):
    qid = answered_while(store, monkeypatch, "working")
    service.deliver(store, store.get(qid), force=True)
    assert len(prompts(fake_herdr)) == 1 and store.get(qid).verified_at


def test_pending_since_survives_ticks_and_clears_on_delivery(store, fake_herdr, monkeypatch, live_session):
    qid = answered_while(store, monkeypatch, "working")
    first = store.get_pending_since(qid)
    service.retry_deliveries(store, "w1:p1")
    assert first and store.get_pending_since(qid) == first
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    service.retry_deliveries(store, "w1:p1")
    assert store.get(qid).verified_at and store.get_pending_since(qid) is None


def test_pending_text_explains_the_wait():
    from herdr_inbox import tui

    assert tui.pending_text("busy: agent is working", 5.0) == "agent busy — will send in 15 s"
    assert tui.pending_text("busy: agent is working", 25.0) == "agent busy — sending at the next retry (every 10 s)"
    assert tui.pending_text("blocked: agent is blocked", 99.0) == "waiting: agent is at a prompt/dialog"
    assert tui.pending_text("redelivery requested", None) == "redelivery requested"


async def test_popup_says_why_and_offers_send_now(store, fake_herdr, monkeypatch, live_session):
    qid = answered_while(store, monkeypatch, "working")
    service.ask(store, B, "other?")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert f"⏳ Q{qid} answered · waiting to deliver: agent busy — will send in" in body
        await pilot.press("down", "enter")
        await pilot.pause()
        fv = app.query_one(FailedView)
        assert "Send now" in str(fv.query_one("#failed-rows").render())
        await pilot.press("1")
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert len(prompts(fake_herdr)) == 1 and store.get(qid).verified_at
