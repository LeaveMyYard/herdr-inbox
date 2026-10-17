"""B1 and its return in the tab view (reviews of #1 and #4): keystrokes meant for one
question never answer another, whatever a poll does underneath."""

import json

from textual.widgets import Input

from herdr_inbox import herdr, service
from herdr_inbox.service import Caller
from herdr_inbox.tui import GoneView, GroupListView, InboxApp, QuestionView

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")


async def test_withdraw_mid_typing_never_answers_the_other_agent(store, fake_herdr):
    """The reviewer's repro, inside a group: Q2 has options, so a swapped view would answer it."""
    service.ask_group(store, B, [{"question": "Q-for-B: which branch name?", "options": ["main", "dev"]},     # Q1
                                 {"question": "Q-also: delete the staging bucket data?", "options": ["yes", "no"]}])
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("3", "enter", *"use feat")   # Q1: Type something.
        service.close(store, 1, "withdrawn", "resolved myself")
        app.poll()
        await pilot.pause()
        assert app.query(GoneView) and "withdrawn by the agent" in str(app.query_one(GoneView).query_one(".question").render())
        await pilot.press(*"ure-x", "enter", "1", "enter")  # keeps typing, Enter, a digit, Enter
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert app.query(GoneView)  # still the inert placeholder
        app.grace_until = 0  # a deliberate move, after the post-close grace period
        await pilot.press("right")  # an explicit move leaves it, and it drops out of the tab row
        assert app.query_one(QuestionView).q.id == 2 and ("q", 1) not in app.tabs
    assert store.get(2).status == "open" and store.get(2).draft_json is None
    assert fake_herdr.calls_of("agent", "prompt") == []


async def test_cursor_on_an_option_when_the_question_closes(store, fake_herdr):
    service.ask_group(store, B, [{"question": "First?", "options": ["x", "y"]},
                                 {"question": "Second?", "options": ["yes", "no"]}])
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.close(store, 1, "answered", "from the CLI")
        app.poll()
        await pilot.pause()
        await pilot.press("enter", "1", "enter")
        await pilot.pause()
    assert store.get(2).status == "open" and fake_herdr.calls_of("agent", "prompt") == []


async def test_several_to_one_does_not_make_enter_send(store, fake_herdr):
    service.ask_group(store, A, [{"question": "Stays?", "options": ["a", "b"]}, {"question": "Goes?", "options": ["c"]}])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        service.close(store, 2, "withdrawn")
        app.poll()
        await pilot.pause()
        await pilot.press("1", "enter")  # still several mode: a draft, then on to Submit
        await pilot.pause()
    assert store.get(1).status == "open" and store.get(1).draft_json["choices"] == ["a"]
    assert fake_herdr.calls_of("agent", "prompt") == []


async def test_new_question_mid_typing_keeps_the_text(store, fake_herdr):
    """M1: other tabs coming and going never rebuild the current view."""
    service.ask(store, A, "Only question?", options=["a", "b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("1", "space", "n", *"note", "enter")  # a note on a
        await pilot.press("3", "enter", *"half typed")
        before = app.query_one(QuestionView)
        service.ask(store, B, "Second?", options=["x"])
        app.poll()
        await pilot.pause()
        service.close(store, 2, "withdrawn")
        app.poll()
        await pilot.pause()
        assert app.query_one(QuestionView) is before and before.notes == {0: "note"}
        assert app.query_one("#inline", Input).value == "half typed"


async def test_go_to_agent_works_for_multi_select(store, fake_herdr):
    service.ask(store, A, "Which?", options=["x", "y"], multi_select=True)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        assert "g go to agent" in str(app.query_one(QuestionView).query_one(".hint").render())
        await pilot.press("g")
        await fake_herdr.went_to_agent(app)
    assert fake_herdr.calls_of("agent", "focus") == [["agent", "focus", "w1:p1"]]


async def test_failed_delivery_entry_retry_and_hide(store, fake_herdr, monkeypatch):
    """⚠ undelivered answers are their own list entries (were tabs before 0.4)."""
    a = service.ask(store, A, "One?")["id"]
    b = service.ask(store, B, "Two?")["id"]
    c = service.ask(store, A, "Three?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, a, "first")
    service.answer(store, b, "second")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "list"
        assert app.entries == [("group", c), ("failed", a), ("failed", b)]  # groups first, ⚠ after
        body = app.query_one(GroupListView).body_text()
        assert "⚠ Q1 not delivered" in body and "⚠ Q2 not delivered" in body
        await pilot.press("2", "enter")
        assert "Q1 not delivered" in str(app.query("#body > *").first().query_one(".question").render())
        monkeypatch.delenv("FAKE_HERDR_FAIL")
        await pilot.press("1")  # Retry delivery: back to the list, the row is inert meanwhile
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert store.get(a).delivered_at and app.mode == "list"
        await pilot.press("3", "enter", "2")  # Q2's entry: stop showing it
        await pilot.pause()
        assert app.mode == "list" and app.closed.get(("failed", b)) == "hidden"
        await pilot.press("up")  # a deliberate move drops the placeholders
        assert ("failed", b) not in app.entries
    assert store.get(b).status == "answered" and store.get(b).delivered_at is None
    assert [q.id for q in store.unacknowledged_answers("aaaaaaaa-0000-4000-8000-00000000000a", "w1:p2")] == [b]  # still for check_answers


async def test_focus_pane_comes_from_the_plugin_context(store, fake_herdr, monkeypatch):
    service.ask(store, A, "From A?")
    service.ask(store, B, "From B?")
    monkeypatch.setenv("HERDR_PLUGIN_CONTEXT_JSON", json.dumps({"focused_pane_id": "w1:p2"}))
    monkeypatch.setenv("INBOX_FOCUS_PANE", "w1:p1")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.focus_pane == "w1:p2" and app.entries[0] == ("group", 2)
        assert app.query_one(GroupListView).cursor == ("group", 2)
    assert herdr.invocation_pane() == "w1:p2"
