"""0.4 group list: one row per ask_operator call, target cursor, inert closed rows."""

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import GroupListView, InboxApp, QuestionView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")
C = Caller("w1:p3", "w1:t3", "tab-C", "sess-A", "/r")


def body(app) -> str:
    return app.query_one(GroupListView).body_text()


async def test_opens_straight_into_the_only_group_but_not_with_a_warning_entry(store, fake_herdr, monkeypatch):
    service.ask_group(store, A, [{"question": "One?"}, {"question": "Two?"}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "group" and app.direct
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    qid = service.ask(store, B, "Old?")["id"]
    service.answer(store, qid, "x")  # undelivered: a ⚠ entry
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "list" and app.entries[-1] == ("failed", qid)


async def test_closed_row_is_inert_until_the_operator_moves(store, fake_herdr):
    service.ask(store, A, "first?", options=["a"])
    service.ask(store, B, "second?", options=["b"])
    service.ask(store, C, "third?", options=["c"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down")  # on B's row
        lv = app.query_one(GroupListView)
        assert lv.cursor == ("group", 2)
        service.close(store, 2, "withdrawn")
        app.poll()
        await pilot.pause()
        assert lv.cursor == ("group", 2) and app.entries == [("group", 1), ("group", 2), ("group", 3)]
        assert "withdrawn by the agent" in body(app)
        await pilot.press("enter", "g", "d", "y")  # all inert on a closed row
        await pilot.pause()
        assert app.mode == "list" and app.is_running and fake_herdr.calls_of("agent", "focus") == []
        await pilot.press("3")  # a digit still names the row it named before the close
        assert lv.cursor == ("group", 3) and ("group", 2) not in app.entries  # the move purges
        await pilot.press("enter")
        assert app.query_one(QuestionView).q.id == 3
    assert [store.get(i).status for i in (1, 3)] == ["open", "open"]


async def test_cursor_target_vanishing_from_a_return_parks_inert(store, fake_herdr):
    service.ask(store, A, "first?", options=["a"])
    service.ask(store, B, "second?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter")  # into A's lone question
        await pilot.press("escape")  # back: cursor still on A
        lv = app.query_one(GroupListView)
        assert app.mode == "list" and lv.cursor == ("group", 1)
        await pilot.press("enter", "1", "enter")  # send A
        await app.workers.wait_for_complete()
        await pilot.pause()
        lv = app.query_one(GroupListView)
        assert lv.cursor == ("group", 1) and app.closed[("group", 1)] == "sending…"
        await pilot.press("enter")  # inert: never re-opens or re-sends
        await pilot.pause()
        assert app.mode == "list"
        await pilot.press("down")
        assert lv.cursor == ("group", 2)
    assert len(fake_herdr.calls_of("agent", "prompt")) == 1


async def test_g_from_the_list_goes_to_the_groups_agent(store, fake_herdr):
    service.ask(store, A, "first?")
    service.ask(store, B, "second?")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down", "g")
        await pilot.pause()
    assert fake_herdr.calls_of("agent", "focus") == [["agent", "focus", "w1:p2"]]


async def test_last_group_sent_closes_the_popup(store, fake_herdr):
    service.ask(store, A, "only?", options=["a"])
    service.ask(store, B, "other?", options=["b"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        for _ in range(2):
            await pilot.press("enter", "1", "enter")  # open, send
            await app.workers.wait_for_complete()
            await pilot.pause()
            if app.is_running and app.mode == "list":
                await pilot.press("down")
        await pilot.pause()
        assert not app.is_running
    assert [store.get(i).status for i in (1, 2)] == ["answered", "answered"]
