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
    """The reviewer's repro: Tab still reaches Submit; the new question is only marked."""
    service.ask(store, A, "q1?", options=["a", "b"])
    service.ask(store, A, "q2?", options=["x", "y"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter", "enter", "shift+tab")  # draft both, back to Q2
        await pilot.pause()
        service.ask(store, B, "drop prod table?", options=["yes", "no"])
        app.poll()
        await pilot.pause()
        assert chips(app)[-1] == "+1 new (press +)" and ("q", 3) not in app.tabs
        await pilot.press("tab", "enter")  # habit: Tab to Submit, Enter submits
        await app.workers.wait_for_complete()
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause(0.3)
        assert app.is_running  # a pending question keeps the popup open
    assert store.get(3).status == "open" and store.get(3).draft_json is None
    prompts = [c[3] for c in fake_herdr.calls_of("agent", "prompt")]
    assert prompts == ["[inbox] answered — Q1 q1?: a · Q2 q2?: x"]


async def test_plus_opens_the_new_questions_deliberately(store, fake_herdr):
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, A, "q2?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.ask(store, B, "new one?", options=["c"])
        service.ask(store, B, "newer?", options=["d"])
        app.poll()
        await pilot.pause()
        assert "+2 new (press +)" in chips(app)
        await pilot.press("plus")
        await pilot.pause()
        assert app.query_one(QuestionView).q.id == 3 and app.tabs[2:4] == [("q", 3), ("q", 4)]
        assert not any(c.startswith("+") for c in chips(app))


async def test_arrival_into_the_empty_view_still_opens_with_tab(store, fake_herdr):
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.ask(store, A, "late?", options=["a"])
        app.poll()
        await pilot.pause()
        await pilot.press("tab")
        assert app.query_one(QuestionView).q.id == 1


async def test_question_view_is_armed_only_after_a_moment(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(tui, "ARM_SECONDS", 0.4)
    service.ask(store, A, "q1?", options=["a", "b"])
    service.ask(store, B, "q2?", options=["x", "y"])
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
