"""Re-review of #4: no background event (poll, arrival, close, delivery result) may change
what the operator's next keypress does."""

import time

from herdr_inbox import service, tui
from herdr_inbox.service import Caller
from herdr_inbox.store import Store
from herdr_inbox.tui import GoneView, InboxApp, QuestionView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")


def text_of(w) -> str:
    return str(w.render())


async def test_arrival_into_the_empty_popup_does_not_take_over(store, fake_herdr):
    """H1 repro: the operator presses Enter to close as a question arrives."""
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.ask(store, A, "delete bucket?", options=["yes", "no"])
        app.poll()
        await pilot.pause()
        assert not app.query(QuestionView)
        assert text_of(app.query_one("#empty")) == "1 new question — press Tab/→ to view"
        await pilot.press("enter")  # "any key closes" still means exactly that
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert not app.is_running
    assert store.get(1).status == "open" and fake_herdr.calls_of("agent", "prompt") == []


async def test_arrival_then_deliberate_tab_enables_immediate_send(store, fake_herdr):
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.ask(store, A, "delete bucket?", options=["yes", "no"])
        app.poll()
        await pilot.pause()
        await pilot.press("tab")  # deliberate
        assert app.query_one(QuestionView).q.id == 1
        await pilot.press("2", "enter")
        await app.workers.wait_for_complete()
    assert store.get(1).answer == "no"


async def test_typing_after_the_question_closes_never_reaches_app_keys(store, fake_herdr):
    """M1 repro: 'use the good one' -- the g must not focus the agent's pane."""
    service.ask(store, B, "branch name?", options=["main", "dev"])
    service.ask(store, A, "delete bucket?", options=["yes", "no"])
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("3", "enter", *"use the ")
        service.close(store, 1, "withdrawn", "x")
        app.poll()
        await pilot.pause()
        await pilot.press(*"good one", "enter", "d", "y")
        await pilot.pause()
        assert app.is_running and app.query(GoneView)
    assert fake_herdr.calls_of("agent", "focus") == []
    assert store.get(2).status == "open"


async def test_keys_are_swallowed_briefly_after_a_background_view_change(store, fake_herdr, monkeypatch):
    service.ask(store, B, "first?", options=["a"])
    service.ask(store, A, "second?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.close(store, 1, "withdrawn")
        app.poll()
        await pilot.press("right")  # within the grace period: swallowed
        await pilot.pause()
        assert app.query(GoneView)
        app.grace_until = time.monotonic() - 1  # the grace period is over
        await pilot.press("right")
        await pilot.pause()
        assert app.query_one(QuestionView).q.id == 2
    assert tui.GRACE_SECONDS == 0.5


async def test_gone_tab_disables_g_and_d(store, fake_herdr):
    service.ask(store, B, "first?", options=["a"])
    service.ask(store, A, "second?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.close(store, 1, "withdrawn")
        app.poll()
        app.grace_until = 0
        await pilot.press("g", "d", "y", "enter", "1")
        await pilot.pause()
        assert app.query(GoneView) and app.is_running and len(app.screen_stack) == 1
    assert fake_herdr.calls_of("agent", "focus") == [] and store.get(2).status == "open"


async def test_dismiss_needs_y(store, fake_herdr):
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, B, "q2?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("d", "d")  # a quick double d is not a confirmation
        await pilot.pause()
        assert store.get(1).status == "open"
        await pilot.press("d", "y")
        await pilot.pause()
    assert store.get(1).status == "dismissed"


def test_delivery_claims_the_row_before_sending(store, fake_herdr, monkeypatch):
    """Retry from a ⚠ tab and the MCP retry loop can never both prompt the same answer."""
    qid = service.ask(store, A, "One?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, qid, "yes")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    other = Store()
    assert other.claim_delivery(qid)  # e.g. the MCP retry thread got there first
    fake_herdr.reset()
    service.retry_deliveries(store, "w1:p1")  # the popup's retry
    service.deliver(store, store.get(qid))
    assert fake_herdr.calls_of("agent", "prompt") == []
    assert not other.claim_delivery(qid)
    # a claim left behind by a process that died is taken over after a while
    store.db.execute("UPDATE questions SET delivery_error = 'sending:2020-01-01T00:00:00+00:00' WHERE id = ?", (qid,))
    service.deliver(store, store.get(qid))
    assert store.get(qid).delivered_at
    assert store.failed_deliveries() == []
