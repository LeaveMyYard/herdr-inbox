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


async def test_a_send_finishing_after_the_popup_exits_does_not_raise(store, fake_herdr, monkeypatch):
    """Review of #16 (Low): a send in flight when the popup exits (e.g. "go to agent") must
    not raise from its background thread when it reports back."""
    from herdr_inbox import tui

    qid = service.ask(store, A, "q?", options=["yes"])["id"]
    service.ask(store, B, "other?")
    app = InboxApp(store, focus_pane="w1:p1")
    captured = []
    monkeypatch.setattr(app, "run_worker", lambda fn, **kw: captured.append(fn))
    async with app.run_test() as pilot:
        await pilot.pause()
        tui._send(app, store, [qid], lambda s, deliver: [s.get(qid)])
    assert captured

    def gone(*a, **k):  # what Textual raises once the app has stopped (App.call_from_thread)
        raise RuntimeError("App is not running")

    monkeypatch.setattr(app, "call_from_thread", gone)
    captured[-1]()  # the send thread (captured[0] is the radar check) reports back late
    assert store.get(qid)  # the send's own work still happened


async def test_poll_does_nothing_once_unmounting(store, fake_herdr, monkeypatch):
    service.ask(store, A, "q?")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        reads = []
        monkeypatch.setattr(app, "_live", lambda: reads.append(1) or ([], []))
        app._unmounted = True  # on_unmount has run; widgets may still be there
        app.poll()
        assert reads == []


async def test_an_error_inside_the_send_report_still_surfaces(store, fake_herdr, monkeypatch):
    """Review of #18: only 'the app is gone' is swallowed, never a real error."""
    import pytest

    from herdr_inbox import tui

    qid = service.ask(store, A, "q?", options=["yes"])["id"]
    app = InboxApp(store, focus_pane="w1:p1")
    captured = []
    monkeypatch.setattr(app, "run_worker", lambda fn, **kw: captured.append(fn))

    def boom(*a, **k):
        raise ValueError("a real bug in _after_submit")

    async with app.run_test() as pilot:
        await pilot.pause()
        tui._send(app, store, [qid], lambda s, deliver: [s.get(qid)])
        monkeypatch.setattr(app, "call_from_thread", lambda fn, *a, **k: boom())
        with pytest.raises(ValueError):
            captured[-1]()  # the app is running: the error must not be hidden


async def test_a_runtime_error_while_the_app_still_runs_is_not_swallowed(store, fake_herdr, monkeypatch):
    import pytest

    from herdr_inbox import tui

    qid = service.ask(store, A, "q?", options=["yes"])["id"]
    app = InboxApp(store, focus_pane="w1:p1")
    captured = []
    monkeypatch.setattr(app, "run_worker", lambda fn, **kw: captured.append(fn))

    def gone_but_not_really(*a, **k):
        raise RuntimeError("something else")

    async with app.run_test() as pilot:
        await pilot.pause()
        tui._send(app, store, [qid], lambda s, deliver: [s.get(qid)])
        monkeypatch.setattr(app, "call_from_thread", gone_but_not_really)
        with pytest.raises(RuntimeError):
            captured[-1]()
