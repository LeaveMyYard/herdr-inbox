"""0.6.0: open the popup on a specific question / group from another plugin.

`herdr plugin pane open` cannot pass arguments to the pane command, so `inbox open --question
Qn | --group G | --pane P | --session S` writes a one-shot focus request (state dir, atomic,
newest wins) and opens the popup; the popup consumes it on start (within REQUEST_TTL)."""

import json
import os
import threading
import time

import pytest

from herdr_inbox import cli, deeplink, service
from herdr_inbox.service import Caller
from herdr_inbox.tui import GroupListView, InboxApp, QuestionView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r")


def three_groups(store):
    g1 = service.ask_group(store, A, [{"question": "a1?"}, {"question": "a2?"}])["group"]
    g2 = service.ask_group(store, B, [{"question": "b1?"}, {"question": "b2?"}, {"question": "b3?"}])["group"]
    g3 = service.ask(store, B, "b-later?")["id"]
    return g1, g2, g3


# -- the request file -----------------------------------------------------------------------


def test_request_is_written_atomically_and_consumed_once(fake_herdr):
    deeplink.write_request("question", "7")
    path = deeplink.request_path()
    data = json.loads(path.read_text())
    assert data["kind"] == "question" and data["value"] == "7" and data["nonce"] and data["created_at"]
    assert not [p for p in path.parent.iterdir() if p.name.startswith(".open-request")]  # no temp left
    assert deeplink.consume_request() == {"kind": "question", "value": "7"}
    assert deeplink.consume_request() is None and not path.exists()


def test_newest_request_wins(fake_herdr):
    deeplink.write_request("question", "1")
    deeplink.write_request("group", "2")
    assert deeplink.consume_request() == {"kind": "group", "value": "2"}


def test_stale_request_is_ignored_and_removed(fake_herdr, monkeypatch):
    deeplink.write_request("question", "1")
    real = time.time
    monkeypatch.setattr(deeplink.time, "time", lambda: real() + deeplink.REQUEST_TTL + 1)
    assert deeplink.consume_request() is None and not deeplink.request_path().exists()


def test_garbage_request_is_ignored(fake_herdr):
    path = deeplink.request_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("{not json")
    assert deeplink.consume_request() is None and not path.exists()


def test_concurrent_popups_consume_a_request_exactly_once(fake_herdr):
    deeplink.write_request("question", "3")
    got, start = [], threading.Barrier(8)

    def consume():
        start.wait()
        got.append(deeplink.consume_request())

    threads = [threading.Thread(target=consume) for _ in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert [g for g in got if g] == [{"kind": "question", "value": "3"}]


# -- resolving the target -------------------------------------------------------------------


def test_targets_resolve_to_a_group_and_question(store, fake_herdr):
    g1, g2, g3 = three_groups(store)
    assert deeplink.resolve(store, {"kind": "question", "value": "Q4"}) == (("group", g2), 4)
    assert deeplink.resolve(store, {"kind": "group", "value": str(g2)}) == (("group", g2), None)
    assert deeplink.resolve(store, {"kind": "pane", "value": "w1:p2"}) == (("group", g2), None)  # oldest
    assert deeplink.resolve(store, {"kind": "session", "value": "sess-A"}) == (("group", g1), None)
    service.close(store, 4, "dismissed")
    assert deeplink.resolve(store, {"kind": "question", "value": "4"}) is None  # gone
    assert deeplink.resolve(store, {"kind": "pane", "value": "w9:p9"}) is None
    assert deeplink.resolve(store, {"kind": "group", "value": "x"}) is None


# -- the popup ------------------------------------------------------------------------------


async def test_popup_opens_on_the_requested_question(store, fake_herdr, monkeypatch):
    from herdr_inbox import tui

    monkeypatch.setattr(tui, "ARM_SECONDS", 0.4)  # the real arming (conftest zeroes it)
    g1, g2, g3 = three_groups(store)
    deeplink.write_request("question", "4")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "group" and app.entry == ("group", g2)
        qv = app.query_one(QuestionView)
        assert qv.q.id == 4 and qv.armed_at > time.monotonic()  # appeared now: arming applies
        await pilot.press("1", "enter")  # keys right after it appeared: ignored
        assert store.get(4).status == "open"
        await pilot.press("escape")  # back to the list (not out of the popup)
        await pilot.pause()
        assert app.mode == "list" and app.is_running


@pytest.mark.parametrize("kind,value", [("group", "3"), ("pane", "w1:p2"), ("session", "sess-B")])
async def test_popup_opens_on_the_group_for_each_target_kind(store, fake_herdr, kind, value):
    g1, g2, g3 = three_groups(store)
    assert g2 == 3
    deeplink.write_request(kind, value)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "group" and app.entry == ("group", g2)
        assert app.query_one(QuestionView).q.id == 3


async def test_gone_target_falls_back_to_the_list_with_a_notice(store, fake_herdr):
    three_groups(store)
    service.close(store, 4, "dismissed")
    deeplink.write_request("question", "4")
    app = InboxApp(store, focus_pane="w1:p1")
    notices = []
    app.notify = lambda message, **kw: notices.append(message)
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "list" and app.query(GroupListView)
    assert notices == ["Q4 is no longer open"]


async def test_no_request_keeps_the_normal_start(store, fake_herdr):
    three_groups(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "list"


# -- the command ----------------------------------------------------------------------------


@pytest.mark.parametrize("args,expected", [
    (["--question", "Q12"], {"kind": "question", "value": "12"}),
    (["--group", "5"], {"kind": "group", "value": "5"}),
    (["--pane", "w6:p3A"], {"kind": "pane", "value": "w6:p3A"}),
    (["--session", "f3740b1a"], {"kind": "session", "value": "f3740b1a"}),
])
def test_open_command_writes_the_request_then_opens_the_popup(fake_herdr, args, expected):
    assert cli.main(["open", *args]) == 0
    assert deeplink.consume_request() == expected
    assert fake_herdr.calls_of("plugin", "pane", "open")


def test_open_command_takes_one_target(fake_herdr):
    with pytest.raises(SystemExit):
        cli.main(["open", "--question", "1", "--pane", "w1:p1"])
    assert not fake_herdr.calls_of("plugin", "pane", "open")
