"""The same safety rules with the REAL delays and realistic key spacing (the other pilot
tests zero the delays to test logic). One test per view type: list, question, Submit, ⚠."""

import pytest

from herdr_inbox import service, tui
from herdr_inbox.service import Caller
from herdr_inbox.tui import FailedView, GroupListView, InboxApp, QuestionView, ReviewView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")
GAP = 0.15  # the reviewer's spacing between keys


@pytest.fixture(autouse=True)
def real_delays(monkeypatch):
    monkeypatch.setattr(tui, "ARM_SECONDS", 0.4)
    monkeypatch.setattr(tui, "ROW_ARM_SECONDS", 1.0)


async def keys(pilot, *names):
    for name in names:
        await pilot.press(name)
        await pilot.pause(GAP)


def prompts(fake):
    return [c[3] for c in fake.calls_of("agent", "prompt")]


async def test_list_wrap_with_real_arming(store, fake_herdr):
    """The reviewer's repro: ↓ Enter Enter Enter right after an arrival never answers it."""
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, B, "q2?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause(0.5)
        await keys(pilot, "down")
        service.ask(store, B, "drop prod?", options=["yes"])
        app.poll()
        await pilot.pause(0.1)
        await keys(pilot, "down", "enter", "enter", "enter")
        await pilot.pause(0.3)
    assert store.get(3).status == "open"
    assert not any("drop prod" in p for p in prompts(fake_herdr))


async def test_list_row_that_just_arrived_is_not_selectable(store, fake_herdr):
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, B, "q2?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause(0.5)
        lv = app.query_one(GroupListView)
        await keys(pilot, "down")
        service.ask(store, B, "fresh?", options=["f"])
        app.poll()
        await keys(pilot, "3")  # a digit onto the row that just appeared: ignored
        assert lv.cursor == ("group", 2)
        await keys(pilot, "down", "enter")  # an arrow may move onto it, Enter does not open it
        assert lv.cursor == ("group", 3) and app.mode == "list"
        await pilot.pause(1.0)
        await keys(pilot, "enter")
        assert app.mode == "group" and app.query_one(QuestionView).q.id == 3


async def test_question_view_ignores_keys_until_armed(store, fake_herdr):
    service.ask(store, A, "q1?", options=["a", "b"])
    service.ask(store, B, "q2?", options=["c"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause(0.5)
        await pilot.press("enter", "2", "enter")  # open, and keys right behind it
        await pilot.pause(0.05)
        assert app.query_one(QuestionView).cursor == 0 and store.get(1).status == "open"
        await pilot.pause(0.45)
        await keys(pilot, "2", "enter")
        await app.workers.wait_for_complete()
    assert store.get(1).answer == "b"


async def test_submit_tab_ignores_keys_until_armed(store, fake_herdr):
    service.ask_group(store, A, [{"question": "q1?", "options": ["a"]}, {"question": "q2?", "options": ["b"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause(0.5)
        await keys(pilot, "tab", "tab", "enter")  # Submit tab reached, Enter at once: ignored
        assert app.query(ReviewView) and app.is_running
        await pilot.pause(0.3)
        await keys(pilot, "down", "enter")  # Cancel, now armed
        await pilot.pause(0.2)
        assert not app.is_running
    assert prompts(fake_herdr) == []


async def test_undelivered_view_ignores_keys_until_armed(store, fake_herdr, monkeypatch):
    qid = service.ask(store, A, "late?", options=["y"])["id"]
    service.ask(store, B, "other?")
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, qid, "y")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause(0.5)
        await keys(pilot, "down", "enter", "1")  # open the ⚠ entry, "Retry" at once: ignored
        assert app.query(FailedView) and store.get(qid).delivered_at is None
        await pilot.pause(0.3)
        await keys(pilot, "1")
        await app.workers.wait_for_complete()
    assert store.get(qid).delivered_at
