"""Final review of #2: cursor stays visible (M1), sends never freeze the popup (L-a),
marker failures never fail ask_operator (L-b)."""

import sqlite3
import threading
import time

from mcp import Client

from herdr_inbox import service, tabs, tui
from herdr_inbox.server import mcp
from herdr_inbox.service import Caller
from herdr_inbox.store import Store
from herdr_inbox.tui import InboxApp, QuestionView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
LONG = "a fairly long description that wraps in a narrow popup " * 2


def visible(app, row_id):
    row, box = app.query_one(row_id), app.query_one(".choices")
    return bool(row.region.height) and box.region.contains_region(row.region)


async def test_cursor_row_stays_visible_in_a_short_popup(store, fake_herdr):
    """The reviewer's repro: 60x20, 9 options with descriptions and previews."""
    opts = [{"label": f"Option {i}", "description": LONG, "preview": f"preview {i}\n" * 3} for i in range(1, 10)]
    service.ask(store, A, "Pick a config for the billing rollout?", options=opts)
    app = InboxApp(store)
    async with app.run_test(size=(60, 20)) as pilot:
        await pilot.pause()
        for _ in range(8):
            await pilot.press("down")
        await pilot.pause()
        assert app.query_one(QuestionView).cursor == 8
        assert visible(app, "#row-8")
        await pilot.press("1")
        await pilot.pause()
        assert visible(app, "#row-0")


async def test_cursor_row_stays_visible_without_previews(store, fake_herdr):
    service.ask(store, A, "Pick one?", options=[{"label": f"Option {i}", "description": LONG} for i in range(1, 6)])
    app = InboxApp(store)
    async with app.run_test(size=(60, 20)) as pilot:
        await pilot.pause()
        await pilot.press("7")  # "Chat about this", the last row
        await pilot.pause()
        assert visible(app, "#row-6")


async def test_send_runs_in_the_background(store, fake_herdr, monkeypatch):
    service.ask(store, A, "One?", options=["a", "b"])
    release = threading.Event()
    real_answer = service.answer

    def slow_answer(s, *args, **kwargs):
        assert threading.current_thread() is not threading.main_thread()
        release.wait(5)  # herdr is slow / the marker lock is busy
        return real_answer(s, *args, **kwargs)

    monkeypatch.setattr(service, "answer", slow_answer)
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r"), "Other?")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter")  # open the lone question from the list
        t0 = time.monotonic()
        await pilot.press("enter")  # pick "a": sends, back to the list at once
        await pilot.press("down")  # the popup keeps reacting while the send is in flight
        await pilot.pause()
        assert time.monotonic() - t0 < 2 and app.mode == "list"
        assert app.query_one(tui.GroupListView).cursor == ("group", 2)
        release.set()
        for _ in range(100):  # the send finishes; with nothing left open the popup closes itself
            if not app.is_running or store.get(1).status == "answered":
                break
            await pilot.pause(0.05)
    assert store.get(1).status == "answered" and store.get(1).answer == "a"


def test_marker_failure_never_fails_ask(store, fake_herdr, monkeypatch):
    def locked(*args, **kwargs):
        raise sqlite3.OperationalError("database is locked")

    monkeypatch.setattr(tabs, "prefix", lambda: "❓")
    monkeypatch.setattr(tabs, "apply", locked)
    r = service.ask(store, A, "Still filed?")
    assert r["status"] == "open" and store.get(r["id"]).repeat_count == 0


async def test_mcp_ask_returns_the_open_receipt_when_markers_fail(migrated_db, fake_herdr, monkeypatch):
    def locked(*args, **kwargs):
        raise sqlite3.OperationalError("database is locked")

    monkeypatch.setattr(tabs, "prefix", lambda: "❓")
    monkeypatch.setattr(tabs, "apply", locked)
    async with Client(mcp) as client:
        r = (await client.call_tool("ask_operator", {"question": "Filed once?"})).structured_content
    assert r["status"] == "open"
    with Store() as s:
        assert s.get(r["id"]).repeat_count == 0 and len(s.list()) == 1


def test_mcp_store_uses_a_short_busy_timeout(migrated_db, fake_herdr):
    from herdr_inbox import server

    with server.tool_store() as s:
        assert s.db.execute("PRAGMA busy_timeout").fetchone()[0] <= 5000


async def test_closing_the_popup_waits_for_an_in_flight_send(store, fake_herdr, monkeypatch):
    service.ask(store, A, "One?", options=["a"])
    real_answer = service.answer

    def slow_answer(s, *args, **kwargs):
        time.sleep(0.5)
        return real_answer(s, *args, **kwargs)

    monkeypatch.setattr(service, "answer", slow_answer)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter", "escape")  # send, then close right away
        await pilot.pause()
    assert store.get(1).status == "answered" and fake_herdr.calls_of("agent", "prompt")
