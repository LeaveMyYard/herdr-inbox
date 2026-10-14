"""B1 (review of PR #1): an answer only ever goes to the question it was written for."""

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp, QuestionView


async def test_withdraw_while_answering_never_answers_another_question(store, fake_herdr):
    a = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
    b = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")
    service.ask(store, b, "Q-for-B: which branch name?")                # Q1 (focused pane: first tab)
    service.ask(store, a, "Q-for-A: delete the staging bucket data?")   # Q2
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.query_one(QuestionView).q.id == 1
        await pilot.press("1", "enter", *"yes")   # Q1's Type something. box
        service.close(store, 1, "withdrawn", "resolved myself")
        app.poll()
        await pilot.pause()
        await pilot.press("enter")
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert store.get(2).status == "open" and store.get(2).draft_json is None, "answer meant for Q1 went to Q2"
    assert store.get(1).status == "withdrawn"
    assert fake_herdr.calls_of("agent", "prompt") == []
