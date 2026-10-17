"""The same safety rules with the REAL delays and realistic key spacing (the other pilot
tests zero the delays to test logic). One test per view type: list, question, Submit, ⚠.

Keys that must be IGNORED are pressed right after the view or row appears (with the
reviewer's 0.15 s spacing, well inside the 0.4 s / 1 s arming). Everything that must be
ACCEPTED waits for the condition it needs (the view mounted, armed, the row settled) with a
generous deadline instead of a fixed sleep, and a key meant for a newly opened view is
sent only once that view is mounted (otherwise the previous view would get it), so a slow CI runner cannot turn a wait that was
just too short into a failure (macOS py3.11 flake on main, 2026-10-14).

The keys that must be IGNORED have the opposite problem: they only test the arming window
if they reach the popup inside it. ``early`` checks that against the app's own stamp of each
key's arrival; when a runner was too slow, the whole scenario runs again on a fresh DB
(``attempts``), and only if every attempt was late does the test fail, saying so (issue
#45). Each window is also checked against a time taken before the view opened (or the row
arrived), so a view that armed at once cannot pass as "too slow"."""

import time

import pytest

from herdr_inbox import service, tui
from herdr_inbox.service import Caller
from herdr_inbox.tui import FailedView, GroupListView, InboxApp, QuestionView, ReviewView, _armed

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")
GAP = 0.15  # the reviewer's spacing between keys
DEADLINE = 15.0
ATTEMPTS = 3  # scenario runs before a runner too slow for the arming window fails the test
# A key is stamped (last_key_at) when it reaches the app, and checked against the window a
# little later, in the view's handler: one stamped this close to the window's end counts as
# late too, so dispatch latency cannot turn a key that legitimately armed into a failure.
DISPATCH_MARGIN = 0.05


class Late(Exception):
    """A key meant for an arming window reached the popup after the window closed."""


async def attempts(scenario, fake, monkeypatch, tmp_path):
    """Run ``scenario(store)`` on a fresh DB (and a fresh herdr call log) until one attempt
    gets its early keys in on time; fail if none of ATTEMPTS did."""
    from herdr_inbox.store import Store

    lates = []
    for n in range(ATTEMPTS):
        monkeypatch.setenv("HERDR_INBOX_DB", str(tmp_path / f"inbox-{n}.db"))
        fake.reset()
        store = Store(migrate=True)
        try:
            return await scenario(store)
        except Late as late:
            lates.append(str(late))
        finally:
            store.close()
    pytest.fail(f"runner too slow for the arming window in all {ATTEMPTS} attempts: {lates}")


@pytest.fixture(autouse=True)
def real_delays(monkeypatch):
    monkeypatch.setattr(tui, "ARM_SECONDS", 0.4)
    monkeypatch.setattr(tui, "ROW_ARM_SECONDS", 1.0)


async def keys(pilot, *names):
    for name in names:
        await pilot.press(name)
        await pilot.pause(GAP)


async def until(pilot, cond, what: str):
    end = time.monotonic() + DEADLINE
    while not cond():
        assert time.monotonic() < end, f"timed out waiting for {what}"
        await pilot.pause(0.02)


async def early(pilot, app, closes: float, *names, gap: float = GAP):
    """Press keys meant to land before ``closes`` (a view's ``armed_at``, a new row's arrival
    + ROW_ARM_SECONDS); the caller then asserts they were ignored. If the last one reached the
    popup (``app.last_key_at``, stamped as each key arrives) within DISPATCH_MARGIN of that
    or later, this attempt could not test the window: Late (``attempts`` runs it again)."""
    for name in names:
        await pilot.press(name)
        await pilot.pause(gap)
    late = app.last_key_at - (closes - DISPATCH_MARGIN)
    if late >= 0:
        raise Late(f"a key reached the popup {late:.2f} s too late")


async def view(pilot, app, cls):
    """The mounted view of type ``cls`` (waits for it)."""
    await until(pilot, lambda: bool(app.query(cls)), cls.__name__)
    return app.query_one(cls)


async def armed(pilot, app, cls):
    """The view of type ``cls``, once it accepts Enter and digits."""
    widget = await view(pilot, app, cls)
    await until(pilot, lambda: _armed(widget), f"{cls.__name__} armed")
    return widget


def prompts(fake):
    return [c[3] for c in fake.calls_of("agent", "prompt")]


async def test_list_wrap_with_real_arming(fake_herdr, monkeypatch, tmp_path):
    """The reviewer's repro: ↓ Enter Enter Enter right after an arrival never answers it."""
    async def scenario(store):
        service.ask(store, A, "q1?", options=["a"])
        service.ask(store, B, "q2?", options=["b"])
        app = InboxApp(store, focus_pane="w1:p1")
        async with app.run_test() as pilot:
            await armed(pilot, app, GroupListView)
            await keys(pilot, "down")
            service.ask(store, B, "drop prod?", options=["yes"])
            before = time.monotonic()
            app.poll()
            arrived = max(app.arrived_at.values())
            assert arrived >= before
            await pilot.pause(0.1)
            await early(pilot, app, arrived + tui.ROW_ARM_SECONDS, "down", "enter", "enter", "enter")
            await app.workers.wait_for_complete()
        assert store.get(3).status == "open"
        assert not any("drop prod" in p for p in prompts(fake_herdr))

    await attempts(scenario, fake_herdr, monkeypatch, tmp_path)


async def test_list_row_that_just_arrived_is_not_selectable(fake_herdr, monkeypatch, tmp_path):
    async def scenario(store):
        service.ask(store, A, "q1?", options=["a"])
        service.ask(store, B, "q2?", options=["b"])
        app = InboxApp(store, focus_pane="w1:p1")
        async with app.run_test() as pilot:
            lv = await armed(pilot, app, GroupListView)
            await keys(pilot, "down")
            service.ask(store, B, "fresh?", options=["f"])
            before = time.monotonic()
            app.poll()
            settles = app.arrived_at[("group", 3)] + tui.ROW_ARM_SECONDS
            assert settles >= before + tui.ROW_ARM_SECONDS
            await early(pilot, app, settles, "3")  # a digit onto the row that just appeared: ignored
            assert lv.cursor == ("group", 2)
            await early(pilot, app, settles, "down", "enter")  # an arrow may move onto it, Enter does not open it
            assert lv.cursor == ("group", 3) and app.mode == "list"
            await until(pilot, lambda: not lv.fresh(("group", 3)), "the new row to settle")
            await keys(pilot, "enter")
            qv = await view(pilot, app, QuestionView)
            assert app.mode == "group" and qv.q.id == 3

    await attempts(scenario, fake_herdr, monkeypatch, tmp_path)


async def test_question_view_ignores_keys_until_armed(fake_herdr, monkeypatch, tmp_path):
    async def scenario(store):
        service.ask(store, A, "q1?", options=["a", "b"])
        service.ask(store, B, "q2?", options=["c"])
        app = InboxApp(store, focus_pane="w1:p1")
        async with app.run_test() as pilot:
            await armed(pilot, app, GroupListView)
            before = time.monotonic()
            await pilot.press("enter")
            qv = await view(pilot, app, QuestionView)  # mounted: keys now reach it, still unarmed
            assert qv.armed_at >= before + tui.ARM_SECONDS
            await early(pilot, app, qv.armed_at, "2", "enter", gap=0)  # keys right behind the open: ignored
            await pilot.pause(0.05)
            assert app.query_one(QuestionView).cursor == 0 and store.get(1).status == "open"
            await armed(pilot, app, QuestionView)
            await keys(pilot, "2", "enter")
            await until(pilot, lambda: store.get(1).status == "answered", "the answer")
            await app.workers.wait_for_complete()
        assert store.get(1).answer == "b"

    await attempts(scenario, fake_herdr, monkeypatch, tmp_path)


async def test_submit_tab_ignores_keys_until_armed(fake_herdr, monkeypatch, tmp_path):
    async def scenario(store):
        service.ask_group(store, A, [{"question": "q1?", "options": ["a"]}, {"question": "q2?", "options": ["b"]}])
        app = InboxApp(store)
        async with app.run_test() as pilot:
            await armed(pilot, app, QuestionView)
            await keys(pilot, "tab")
            before = time.monotonic()
            await pilot.press("tab")  # no GAP after it: the window opens with this key
            rv = await view(pilot, app, ReviewView)
            assert rv.armed_at >= before + tui.ARM_SECONDS
            await early(pilot, app, rv.armed_at, "enter")  # Submit tab reached, Enter at once: ignored
            assert app.query(ReviewView) and app.is_running
            await armed(pilot, app, ReviewView)
            await keys(pilot, "down", "enter")  # Cancel, now armed
            await until(pilot, lambda: not app.is_running, "the popup to close")
        assert prompts(fake_herdr) == []

    await attempts(scenario, fake_herdr, monkeypatch, tmp_path)


async def test_undelivered_view_ignores_keys_until_armed(fake_herdr, monkeypatch, tmp_path):
    async def scenario(store):
        qid = service.ask(store, A, "late?", options=["y"])["id"]
        service.ask(store, B, "other?")
        monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
        service.answer(store, qid, "y")
        monkeypatch.delenv("FAKE_HERDR_FAIL")
        app = InboxApp(store, focus_pane="w1:p2")
        async with app.run_test() as pilot:
            await armed(pilot, app, GroupListView)
            await keys(pilot, "down")
            before = time.monotonic()
            await pilot.press("enter")  # no GAP after it: the window opens with this key
            fv = await view(pilot, app, FailedView)  # mounted (a slow runner may take longer than GAP)
            assert fv.armed_at >= before + tui.ARM_SECONDS
            await early(pilot, app, fv.armed_at, "1")  # "Retry" right after it opened: ignored
            assert app.query(FailedView) and store.get(qid).delivered_at is None
            await armed(pilot, app, FailedView)
            await keys(pilot, "1")
            await until(pilot, lambda: store.get(qid).delivered_at is not None, "the retry")
            await app.workers.wait_for_complete()
        assert store.get(qid).delivered_at

    await attempts(scenario, fake_herdr, monkeypatch, tmp_path)
