"""The same safety rules with the REAL delays and realistic key spacing (the other pilot
tests zero the delays to test logic). One test per view type: list, question, Submit, ⚠.

Keys that must be IGNORED are pressed right after the view or row appears (with the
reviewer's 0.15 s spacing, well inside the 0.4 s / 1 s arming). Everything that must be
ACCEPTED waits for the condition it needs (the view mounted, armed, the row settled) with a
generous deadline instead of a fixed sleep, and a key meant for a newly opened view is
sent only once that view is mounted (otherwise the previous view would get it), so a slow CI runner cannot turn a wait that was
just too short into a failure (macOS py3.11 flake on main, 2026-10-14)."""

import time

import pytest

from herdr_inbox import service, tui
from herdr_inbox.service import Caller
from herdr_inbox.tui import FailedView, GroupListView, InboxApp, QuestionView, ReviewView, _armed

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")
GAP = 0.15  # the reviewer's spacing between keys
DEADLINE = 15.0


@pytest.fixture(autouse=True)
def real_delays(monkeypatch):
    monkeypatch.setattr(tui, "ARM_SECONDS", 0.4)
    monkeypatch.setattr(tui, "ROW_ARM_SECONDS", 1.0)


async def keys(pilot, *names):
    for name in names:
        await pilot.press(name)
        await pilot.pause(GAP)


async def until(pilot, cond, what: str):
    end = time.monotonic() + DEADLINE
    while not cond():
        assert time.monotonic() < end, f"timed out waiting for {what}"
        await pilot.pause(0.02)


async def view(pilot, app, cls):
    """The mounted view of type ``cls`` (waits for it)."""
    await until(pilot, lambda: bool(app.query(cls)), cls.__name__)
    return app.query_one(cls)


async def armed(pilot, app, cls):
    """The view of type ``cls``, once it accepts Enter and digits."""
    widget = await view(pilot, app, cls)
    await until(pilot, lambda: _armed(widget), f"{cls.__name__} armed")
    return widget


def prompts(fake):
    return [c[3] for c in fake.calls_of("agent", "prompt")]


async def test_list_wrap_with_real_arming(store, fake_herdr):
    """The reviewer's repro: ↓ Enter Enter Enter right after an arrival never answers it."""
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, B, "q2?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await armed(pilot, app, GroupListView)
        await keys(pilot, "down")
        service.ask(store, B, "drop prod?", options=["yes"])
        app.poll()
        await pilot.pause(0.1)
        await keys(pilot, "down", "enter", "enter", "enter")
        await app.workers.wait_for_complete()
    assert store.get(3).status == "open"
    assert not any("drop prod" in p for p in prompts(fake_herdr))


async def test_list_row_that_just_arrived_is_not_selectable(store, fake_herdr):
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, B, "q2?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        lv = await armed(pilot, app, GroupListView)
        await keys(pilot, "down")
        service.ask(store, B, "fresh?", options=["f"])
        app.poll()
        await keys(pilot, "3")  # a digit onto the row that just appeared: ignored
        assert lv.cursor == ("group", 2)
        await keys(pilot, "down", "enter")  # an arrow may move onto it, Enter does not open it
        assert lv.cursor == ("group", 3) and app.mode == "list"
        await until(pilot, lambda: not lv.fresh(("group", 3)), "the new row to settle")
        await keys(pilot, "enter")
        qv = await view(pilot, app, QuestionView)
        assert app.mode == "group" and qv.q.id == 3


async def test_question_view_ignores_keys_until_armed(store, fake_herdr):
    service.ask(store, A, "q1?", options=["a", "b"])
    service.ask(store, B, "q2?", options=["c"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await armed(pilot, app, GroupListView)
        await pilot.press("enter")
        await view(pilot, app, QuestionView)  # mounted: keys now reach it, still unarmed
        await pilot.press("2", "enter")  # keys right behind the open: ignored
        await pilot.pause(0.05)
        assert app.query_one(QuestionView).cursor == 0 and store.get(1).status == "open"
        await armed(pilot, app, QuestionView)
        await keys(pilot, "2", "enter")
        await until(pilot, lambda: store.get(1).status == "answered", "the answer")
        await app.workers.wait_for_complete()
    assert store.get(1).answer == "b"


async def test_submit_tab_ignores_keys_until_armed(store, fake_herdr):
    service.ask_group(store, A, [{"question": "q1?", "options": ["a"]}, {"question": "q2?", "options": ["b"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await armed(pilot, app, QuestionView)
        await keys(pilot, "tab", "tab")
        await view(pilot, app, ReviewView)
        await keys(pilot, "enter")  # Submit tab reached, Enter at once: ignored
        assert app.query(ReviewView) and app.is_running
        await armed(pilot, app, ReviewView)
        await keys(pilot, "down", "enter")  # Cancel, now armed
        await until(pilot, lambda: not app.is_running, "the popup to close")
    assert prompts(fake_herdr) == []


async def test_undelivered_view_ignores_keys_until_armed(store, fake_herdr, monkeypatch):
    qid = service.ask(store, A, "late?", options=["y"])["id"]
    service.ask(store, B, "other?")
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, qid, "y")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await armed(pilot, app, GroupListView)
        await keys(pilot, "down", "enter")
        await view(pilot, app, FailedView)  # mounted (a slow runner may take longer than GAP)
        await keys(pilot, "1")  # "Retry" right after it opened: ignored
        assert app.query(FailedView) and store.get(qid).delivered_at is None
        await armed(pilot, app, FailedView)
        await keys(pilot, "1")
        await until(pilot, lambda: store.get(qid).delivered_at is not None, "the retry")
        await app.workers.wait_for_complete()
    assert store.get(qid).delivered_at
