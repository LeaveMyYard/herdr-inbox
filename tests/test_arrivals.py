"""Last review of #4: an arrival never changes where Tab goes; a question view is armed
only ~400 ms after it appears; orphaned answers show as ⚠; claims respect check_answers."""

import time

import pytest

from herdr_inbox import service, tui
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp, QuestionView, ReviewView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")


def chips(app):
    return [str(c.render()).strip() for c in app.query("#chips > Static")]


async def test_tab_enter_after_arrival_never_answers_the_new_question(store, fake_herdr):
    """The reviewer's repro, per group: nothing is ever added to an open group, so Tab still
    reaches its Submit and the arrival waits in the list."""
    service.ask_group(store, A, [{"question": "q1?", "options": ["a", "b"]},
                                 {"question": "q2?", "options": ["x", "y"]}])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "group"  # the only group: opened straight away
        await pilot.press("enter", "enter", "shift+tab")  # draft both, back to Q2
        await pilot.pause()
        service.ask(store, B, "drop prod table?", options=["yes", "no"])
        app.poll()
        await pilot.pause()
        assert ("q", 3) not in app.tabs and not any("+" in c for c in chips(app))
        await pilot.press("tab", "enter")  # habit: Tab to Submit, Enter submits
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert app.mode == "list" and app.list_cursor == ("group", 1)  # back to the list
        await pilot.press("enter")  # on the sent group's inert row: nothing
        await pilot.pause(0.3)
        assert app.is_running
    assert store.get(3).status == "open" and store.get(3).draft_json is None
    prompts = [c[3] for c in fake_herdr.calls_of("agent", "prompt")]
    assert prompts == ["[inbox] answered — Q1 q1?: a · Q2 q2?: x"]


async def test_arrivals_are_list_rows_and_never_move_the_cursor(store, fake_herdr):
    """Was the '+N new' marker: the group list makes it redundant. Arrivals are appended as
    rows; the cursor is a target (a group), so it stays where it was."""
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, A, "q2?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "list"
        await pilot.press("down")
        lv = app.query_one(tui.GroupListView)
        assert lv.cursor == ("group", 2)
        service.ask(store, B, "new one?", options=["c"])
        service.ask(store, B, "newer?", options=["d"])
        app.poll()
        await pilot.pause()
        assert app.entries == [("group", 1), ("group", 2), ("group", 3), ("group", 4)]
        assert lv.cursor == ("group", 2) and "+" not in lv.body_text()
        await pilot.press("enter")
        assert app.mode == "group" and app.query_one(QuestionView).q.id == 2


async def test_arrival_into_the_empty_view_still_opens_with_tab(store, fake_herdr):
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.ask(store, A, "late?", options=["a"])
        app.poll()
        await pilot.pause()
        assert "1 new question group" in str(app.query_one("#empty").render())
        await pilot.press("tab")  # deliberate: the list of what arrived
        assert app.mode == "list"
        await pilot.press("enter")
        assert app.query_one(QuestionView).q.id == 1


async def test_question_view_is_armed_only_after_a_moment(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(tui, "ARM_SECONDS", 0.4)
    service.ask_group(store, A, [{"question": "q1?", "options": ["a", "b"]},
                                 {"question": "q2?", "options": ["x", "y"]}])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("2", "enter")  # keys already on their way when the view appeared
        await pilot.pause()
        v = app.query_one(QuestionView)
        assert v.q.id == 1 and v.cursor == 0 and store.get(1).draft_json is None
        time.sleep(0.45)
        await pilot.press("2", "enter")
        await pilot.pause()
        assert store.get(1).draft_json["choices"] == ["b"]
        await pilot.press("enter")  # Q2 was reached just now: not armed yet
        await pilot.pause()
        assert store.get(2).draft_json is None and app.query_one(QuestionView).q.id == 2
    assert tui.ARM_SECONDS == 0.4


async def test_list_and_lone_question_are_armed_too(store, fake_herdr, monkeypatch):
    """Enter opens a lone question from the list; a second Enter right behind it must not send."""
    monkeypatch.setattr(tui, "ARM_SECONDS", 0.4)
    service.ask(store, A, "lone?", options=["yes", "no"])
    service.ask(store, B, "other?", options=["x"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter")  # the list itself just appeared: ignored
        await pilot.pause()
        assert app.mode == "list"
        time.sleep(0.45)
        await pilot.press("enter", "enter")  # open, and an Enter right behind it
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert app.mode == "group" and store.get(1).status == "open"
        time.sleep(0.45)
        await pilot.press("enter")  # now a deliberate answer: sends
        await app.workers.wait_for_complete()
    assert store.get(1).answer == "yes"


def test_orphaned_answer_shows_as_failed(store, fake_herdr):
    qid = service.ask(store, A, "One?")["id"]
    store.set_answer(qid, "yes")  # the answering process died before claiming/delivering
    assert store.failed_deliveries() == []  # maybe still in flight
    store.db.execute("UPDATE questions SET answered_at = '2020-01-01T00:00:00+00:00' WHERE id = ?", (qid,))
    assert [q.id for q in store.failed_deliveries()] == [qid]


def test_claim_refuses_answers_the_agent_already_fetched(store, fake_herdr, monkeypatch):
    qid = service.ask(store, A, "One?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, qid, "yes")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    store.acknowledge([qid])  # the agent ran check_answers
    assert store.claim_delivery(qid) is None
    fake_herdr.reset()
    service.deliver(store, store.get(qid))
    assert fake_herdr.calls_of("agent", "prompt") == [] and store.failed_deliveries() == []
