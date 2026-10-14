"""B1 (reviewer, PR #1): the answer box is bound to the question it was opened for."""

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp


async def test_withdraw_while_typing_answers_other_question(store, fake_herdr):
    a = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
    b = Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r")
    service.ask(store, a, "Q-for-A: delete the staging bucket data?")   # Q1
    service.ask(store, b, "Q-for-B: which branch name?")                # Q2 (top row)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("a")            # operator starts answering Q2 (cursor row 0)
        await pilot.press(*"yes")
        # agent B withdraws Q2 while the operator is typing; the list refreshes
        service.close(store, 2, "withdrawn", "resolved myself")
        app.refresh_list()
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
    assert store.get(1).status == "open", "answer meant for Q2 was applied to Q1"
    assert store.get(2).status == "withdrawn"
    assert fake_herdr.calls_of("agent", "prompt") == []


async def test_poll_does_not_refresh_while_answering(store, fake_herdr):
    service.ask(store, Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r"), "first?")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("a")
        service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "second?")
        app.poll()
        assert len(app.questions) == 1  # rows did not move under the operator
        await pilot.press("escape")
        app.poll()
        assert len(app.questions) == 2
