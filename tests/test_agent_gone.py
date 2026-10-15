"""Answers whose agent is gone, hidden answers, and answers waiting to be delivered.

Live (0.6.2): the popup listed ⚠ rows the operator could not act on -- Q77 (its pane had
closed: "no live agent"), Q75 (pending: agent busy), Q4-6 (pane closed since yesterday,
hidden) -- and the worker polled `agent get` for the gone panes every tick."""

import json
import os
from pathlib import Path

import pytest

from herdr_inbox import service, worker
from herdr_inbox.service import Caller
from herdr_inbox.tui import FailedView, GroupListView, InboxApp

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r")


def gone_answer(store, monkeypatch, text="Gone?"):
    qid = service.ask(store, A, text, options=["yes"])["id"]
    monkeypatch.setenv("FAKE_HERDR_AGENT", "0")  # the pane has no agent any more
    service.answer(store, qid, "yes")
    return qid


def test_worker_marks_a_gone_agent_once_notifies_once_then_stops_polling(store, fake_herdr, monkeypatch):
    qid = gone_answer(store, monkeypatch)
    for _ in range(service.MAX_STALE_RETRIES + 3):
        worker.tick(store)
    assert store.agent_gone(qid)
    fake_herdr.reset()
    for _ in range(5):
        worker.tick(store)
    assert not fake_herdr.calls_of("agent", "get")  # no more polling
    gone = [c for c in fake_herdr.calls_of("notification", "show")]
    q = store.get(qid)
    assert q.acknowledged_at is None and q.status == "answered"  # kept for check_answers


def test_gone_notification_says_the_answer_is_kept(store, fake_herdr, monkeypatch):
    qid = gone_answer(store, monkeypatch)
    for _ in range(service.MAX_STALE_RETRIES + 3):
        worker.tick(store)
    shows = [c for c in fake_herdr.calls_of("notification", "show") if "gone" in c[2]]
    assert len(shows) == 1 and f"Q{qid}" in shows[0][2] and "check_answers" in shows[0][4]


def test_hidden_rows_are_not_polled(store, fake_herdr, monkeypatch):
    qid = gone_answer(store, monkeypatch)
    store.hide_failed(qid)
    fake_herdr.reset()
    for _ in range(3):
        worker.tick(store)
    assert not fake_herdr.calls_of("agent", "get")


def test_legacy_rows_of_a_gone_pane_are_not_polled_every_tick(store, fake_herdr, monkeypatch):
    """The live log: `agent get w6:p3T` -> agent_not_found on every tick."""
    qid = service.ask(store, A, "Old?", options=["yes"])["id"]
    store.db.execute("UPDATE question_ext SET herdr_server = NULL WHERE id = ?", (qid,))
    monkeypatch.setenv("FAKE_HERDR_AGENT", "0")
    service.answer(store, qid, "yes")
    fake_herdr.reset()
    for _ in range(20):
        worker.tick(store)
    assert len(fake_herdr.calls_of("agent", "get")) <= 1  # backed off, not once per tick


async def test_popup_labels_a_gone_row_and_d_hides_it(store, fake_herdr, monkeypatch):
    qid = gone_answer(store, monkeypatch)
    store.mark_agent_gone(qid, "no live agent in pane w1:p1")
    service.ask(store, B, "Other?")
    monkeypatch.setenv("FAKE_HERDR_AGENT", "1")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert f"Q{qid} agent closed — answer kept for check_answers" in body
        await pilot.press("down", "d")  # one key, no confirmation: nothing is lost
        await pilot.pause()
        assert store.db.execute("SELECT hidden_at FROM question_ext WHERE id = ?", (qid,)).fetchone()[0]


async def test_popup_does_not_list_hidden_rows(store, fake_herdr, monkeypatch):
    qid = gone_answer(store, monkeypatch)
    store.hide_failed(qid)
    service.ask(store, B, "Other?")
    service.ask(store, B, "Another?")  # two groups: the popup opens on the list
    monkeypatch.setenv("FAKE_HERDR_AGENT", "1")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert f"Q{qid}" not in app.query_one(GroupListView).body_text()


async def test_pending_rows_say_they_are_already_answered(store, fake_herdr, monkeypatch):
    qid = service.ask(store, A, "Busy?", options=["yes"])["id"]
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.answer(store, qid, "yes")
    service.ask(store, B, "Other?")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert f"Q{qid} answered · waiting to deliver" in body
        await pilot.press("down", "enter")
        await pilot.pause()
        head = str(app.query_one(FailedView).query_one(".question").render())
        assert "already answered" in head
