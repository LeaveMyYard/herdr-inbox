"""A poll that lands while the popup is being torn down must not crash it.

CI (macOS py3.11, #16): the poll timer fired during run_test's teardown, after the screen's
widgets were removed: render_group_title -> NoMatches('#group-title'). Timing decides
whether the last tick lands in that window, so it is a product bug, not a test flake."""

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r")


async def test_poll_after_the_widgets_are_gone_is_a_no_op(store, fake_herdr):
    service.ask_group(store, A, [{"question": "q1?"}, {"question": "q2?"}])
    service.ask(store, B, "other?")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter")  # into a group: the view whose poll renders the title
        await pilot.pause()
        assert app.mode == "group"
    app.poll()  # the late tick: widgets already removed
