"""Third re-review of #4: cursors are target identities, re-resolved after any reload;
claims cover every delivery path."""

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.store import Store
from herdr_inbox.tui import InboxApp, ReviewView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")


async def submit_tab(app, pilot):
    await pilot.pause()
    await pilot.press("enter")  # Q1 -> draft "a", moves to Q2
    await pilot.press("tab")    # Submit tab
    await pilot.pause()
    return app.query_one(ReviewView)


async def test_cancel_stays_cancel_when_a_question_arrives(store, fake_herdr):
    """The reviewer's repro."""
    service.ask(store, A, "q1?", options=["a", "b"])
    service.ask(store, B, "q2?", options=["x", "y"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        v = await submit_tab(app, pilot)
        await pilot.press("down")  # Submit answers -> Cancel
        assert v.cursor == "cancel"
        service.ask(store, A, "q3 new?", options=["m"])
        app.poll()
        await pilot.pause()
        assert v.cursor == "cancel" and len(v.items) == 2 and app.pending == [("q", 3)]
        await pilot.press("enter")
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert store.get(1).status == "open" and fake_herdr.calls_of("agent", "prompt") == []


async def test_item_cursor_follows_its_question(store, fake_herdr):
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, B, "q2?", options=["x"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        v = await submit_tab(app, pilot)
        await pilot.press("up")  # Submit answers -> Q2 row
        assert v.cursor == ("item", 2)
        service.ask(store, A, "q0 earlier tab?")  # lands before Q2 in the list
        app.poll()
        await pilot.pause()
        assert v.cursor == ("item", 2)
        await pilot.press("enter")  # edits Q2, not the row that moved under the cursor
        assert app.current == ("q", 2)


async def test_vanished_target_parks_on_nothing(store, fake_herdr):
    service.ask(store, A, "q1?", options=["a"])
    service.ask(store, B, "q2?", options=["x"])
    service.ask(store, B, "q3?", options=["y"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter", "tab", "tab")  # Q1 drafted; past Q2, Q3 to Submit
        await pilot.pause()
        v = app.query_one(ReviewView)
        await pilot.press("up")  # -> Q3 row
        assert v.cursor == ("item", 3)
        service.close(store, 3, "withdrawn")
        app.poll()
        await pilot.pause()
        assert v.cursor is None and "❯" not in str(v.query_one("#review-body").render())
        await pilot.press("enter")  # inert: neither submit nor cancel nor edit
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert app.is_running and app.current == ("submit", 0)
        await pilot.press("down")  # arrows pick up again from the top
        assert v.cursor == ("item", 1)
    assert store.get(1).status == "open" and fake_herdr.calls_of("agent", "prompt") == []


def test_stranded_claim_shows_as_failed(store, fake_herdr):
    qid = service.ask(store, A, "One?")["id"]
    store.set_answer(qid, "yes")
    store.db.execute("UPDATE questions SET delivery_error = 'sending:2020-01-01T00:00:00+00:00:dead' WHERE id = ?",
                     (qid,))
    assert [q.id for q in store.failed_deliveries()] == [qid]
    assert store.claim_delivery(qid)  # taken over
    assert store.failed_deliveries() == []  # a fresh claim is in flight, not failed


def test_error_paths_never_overwrite_someone_elses_claim(store, fake_herdr, monkeypatch):
    qid = service.ask(store, A, "One?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, qid, "yes")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    claim = Store().claim_delivery(qid)  # another process is mid-send
    monkeypatch.setenv("FAKE_HERDR_AGENT", "0")  # this process would record "no live agent"
    service.deliver(store, store.get(qid))
    monkeypatch.setenv("FAKE_HERDR_AGENT", "1")
    monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-other")  # ... or "conversation changed"
    service.deliver(store, store.get(qid))
    assert store.get(qid).delivery_error == claim


def test_retry_that_backs_off_releases_its_claim(store, fake_herdr, monkeypatch):
    qid = service.ask(store, A, "One?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, qid, "yes")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    before = store.get(qid).delivery_error
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "1")  # the operator is in that pane: wait
    service.retry_deliveries(store, "w1:p1")
    assert store.get(qid).delivery_error == before  # not left claimed
    assert [q.id for q in store.failed_deliveries()] == [qid]
