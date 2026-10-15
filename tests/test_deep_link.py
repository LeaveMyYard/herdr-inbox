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


@pytest.fixture(autouse=True)
def no_nonce(monkeypatch):
    monkeypatch.delenv("INBOX_OPEN_NONCE", raising=False)


def take(nonce):
    os.environ["INBOX_OPEN_NONCE"] = nonce
    return deeplink.consume_request()


def test_request_is_written_atomically_and_consumed_once(fake_herdr):
    nonce = deeplink.write_request("question", "7")
    path = deeplink.request_path()
    data = json.loads(path.read_text())
    assert data["kind"] == "question" and data["value"] == "7" and data["nonce"] and data["created_at"]
    assert not [p for p in path.parent.iterdir() if p.name.startswith(".open-request")]  # no temp left
    assert take(nonce) == {"kind": "question", "value": "7"}
    assert take(nonce) is None and not path.exists()


def test_newest_request_wins(fake_herdr):
    deeplink.write_request("question", "1")
    nonce = deeplink.write_request("group", "2")
    assert take(nonce) == {"kind": "group", "value": "2"}


def test_stale_request_is_ignored_and_removed(fake_herdr, monkeypatch):
    nonce = deeplink.write_request("question", "1")
    real = time.time
    monkeypatch.setattr(deeplink.time, "time", lambda: real() + deeplink.REQUEST_TTL + 1)
    assert take(nonce) is None and not deeplink.request_path().exists()


def test_garbage_request_is_ignored(fake_herdr):
    path = deeplink.request_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("{not json")
    assert take("x") is None and not path.exists()


def test_concurrent_popups_consume_a_request_exactly_once(fake_herdr):
    os.environ["INBOX_OPEN_NONCE"] = deeplink.write_request("question", "3")
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
    os.environ["INBOX_OPEN_NONCE"] = deeplink.write_request("question", "4")
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
    os.environ["INBOX_OPEN_NONCE"] = deeplink.write_request(kind, value)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "group" and app.entry == ("group", g2)
        assert app.query_one(QuestionView).q.id == 3


async def test_gone_target_falls_back_to_the_list_with_a_notice(store, fake_herdr):
    three_groups(store)
    service.close(store, 4, "dismissed")
    os.environ["INBOX_OPEN_NONCE"] = deeplink.write_request("question", "4")
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
    assert take(json.loads(deeplink.request_path().read_text())["nonce"]) == expected
    assert fake_herdr.calls_of("plugin", "pane", "open")


def test_open_command_takes_one_target(fake_herdr):
    with pytest.raises(SystemExit):
        cli.main(["open", "--question", "1", "--pane", "w1:p1"])
    assert not fake_herdr.calls_of("plugin", "pane", "open")


# -- review of #14 --------------------------------------------------------------------------

ROOT = __import__("pathlib").Path(__file__).resolve().parents[1]


def test_open_from_another_plugins_action_opens_the_inbox(fake_herdr, monkeypatch):
    """H1: herdr sets HERDR_PLUGIN_ID/ROOT for the CALLER's action (herdr-panorama)."""
    monkeypatch.setenv("HERDR_PLUGIN_ID", "leavemyyard.herdr-panorama")
    monkeypatch.setenv("HERDR_PLUGIN_ROOT", "/plugins/panorama")
    assert cli.main(["open", "--question", "Q5"]) == 0
    call = fake_herdr.calls_of("plugin", "pane", "open")[-1]
    assert call[call.index("--plugin") + 1] == "leavemyyard.herdr-inbox"
    assert call[call.index("--cwd") + 1] == str(ROOT)
    assert "/plugins/panorama" not in call


def test_cli_rejects_a_malformed_target(fake_herdr):
    for bad in (["--question", "abc"], ["--question", "Q"], ["--group", "x"], ["--group", "Q3"]):
        with pytest.raises(SystemExit):
            cli.main(["open", *bad])
    assert not deeplink.request_path().exists() and not fake_herdr.calls_of("plugin", "pane", "open")


def test_a_manual_open_does_not_take_someone_elses_request(store, fake_herdr, monkeypatch):
    """Low: only the popup `inbox open` started (its nonce) takes the request."""
    nonce = deeplink.write_request("question", "1")
    monkeypatch.delenv("INBOX_OPEN_NONCE", raising=False)
    assert deeplink.consume_request() is None
    monkeypatch.setenv("INBOX_OPEN_NONCE", "someone-else")
    assert deeplink.consume_request() is None
    monkeypatch.setenv("INBOX_OPEN_NONCE", nonce)
    assert deeplink.consume_request() == {"kind": "question", "value": "1"}


def test_the_nonce_is_passed_to_the_popup(fake_herdr):
    assert cli.main(["open", "--group", "2"]) == 0
    call = fake_herdr.calls_of("plugin", "pane", "open")[-1]
    envs = [call[i + 1] for i, a in enumerate(call) if a == "--env"]
    nonce = json.loads(deeplink.request_path().read_text())["nonce"]
    assert f"INBOX_OPEN_NONCE={nonce}" in envs


def test_open_reports_a_popup_that_is_already_open(fake_herdr, capsys):
    with deeplink.popup_running():
        assert cli.main(["open", "--question", "1"]) == 3
        assert "already open" in capsys.readouterr().err
    assert not deeplink.request_path().exists() and not fake_herdr.calls_of("plugin", "pane", "open")
    assert cli.main(["open", "--question", "1"]) == 0  # released with the popup


async def test_the_popup_marks_itself_running(store, fake_herdr):
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert deeplink.popup_is_running()
    assert not deeplink.popup_is_running()


async def test_a_deep_linked_view_waits_for_a_deliberate_key(store, fake_herdr, monkeypatch):
    """M1 (+ re-review): a deep link may land under the operator's typing. Nothing sends until
    Tab/→ -- never ↑/↓, which a shell's history uses -- pressed at least RELEASE_GRACE after
    the popup appeared; the releasing key itself does nothing else."""
    from herdr_inbox import tui

    monkeypatch.setattr(tui, "RELEASE_GRACE", 30.0)  # long enough for a slow runner's key presses
    qid = service.ask(store, A, "solo?", options=["yes", "no"])["id"]
    monkeypatch.setenv("INBOX_OPEN_NONCE", deeplink.write_request("question", str(qid)))
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert "press Tab or →" in str(app.query_one("#held-hint").render())
        await pilot.press("enter", "1", "2", "enter")
        await pilot.press("up", "enter")  # the reviewer's case: shell history under the popup
        await pilot.press("down", "j", "k", "enter")
        await pilot.press("tab", "enter")  # Tab, but within RELEASE_GRACE of appearing: ignored
        await pilot.pause()
        assert store.get(qid).status == "open" and app.held and app.is_running
        app.held_at -= 30.0  # the grace has passed
        await pilot.press("tab")  # deliberate: releases (and does nothing else)
        assert not app.held and app.query_one(QuestionView).q.id == qid
        await pilot.press("enter")
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert store.get(qid).status == "answered"


# -- --no-hold (operator 2026-10-15: the caller decides) -----------------------------------


async def opened(store, no_hold):
    qid = service.ask(store, A, f"solo {no_hold}?", options=["yes", "no"])["id"]
    os.environ["INBOX_OPEN_NONCE"] = deeplink.write_request("question", str(qid), hold=not no_hold)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        held, hint = app.held, app.has_class("held")
        await pilot.press("enter")  # arming is zeroed in tests: Enter answers unless held
        await app.workers.wait_for_complete()
        await pilot.pause()
    return held, hint, store.get(qid).status


async def test_without_no_hold_the_popup_stays_held(store, fake_herdr):
    assert await opened(store, no_hold=False) == (True, True, "open")


async def test_no_hold_opens_like_a_normal_popup(store, fake_herdr):
    assert await opened(store, no_hold=True) == (False, False, "answered")


async def test_no_hold_still_arms(store, fake_herdr, monkeypatch):
    from herdr_inbox import tui

    monkeypatch.setattr(tui, "ARM_SECONDS", 0.4)
    qid = service.ask(store, A, "armed?", options=["yes", "no"])["id"]
    os.environ["INBOX_OPEN_NONCE"] = deeplink.write_request("question", str(qid), hold=False)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter")  # inside the 0.4 s arming: ignored
        assert store.get(qid).status == "open" and not app.held


@pytest.mark.parametrize("flag,hold", [([], True), (["--no-hold"], False)])
def test_open_command_no_hold_flag(fake_herdr, flag, hold):
    assert cli.main(["open", "--question", "Q1", *flag]) == 0
    data = json.loads(deeplink.request_path().read_text())
    assert data.get("hold", True) is hold
