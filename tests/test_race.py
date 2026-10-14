"""B1 (review of PR #1): an answer only ever goes to the question it was written for."""

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp


async def test_withdraw_while_answering_never_answers_another_question(store, fake_herdr):
    a = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
    b = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")
    service.ask(store, a, "Q-for-A: delete the staging bucket data?")   # Q1
    service.ask(store, b, "Q-for-B: which branch name?")                # Q2 (top row)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter", "enter", *"yes")   # Q2's free-text box
        service.close(store, 2, "withdrawn", "resolved myself")
        app.refresh_list()
        await pilot.pause()
        await pilot.press("enter", "enter")             # draft + Submit
        await pilot.pause()
    assert store.get(1).status == "open", "answer meant for Q2 was applied to Q1"
    assert store.get(2).status == "withdrawn"
    assert fake_herdr.calls_of("agent", "prompt") == []
