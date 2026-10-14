"""Regression tests for the PR #1 re-review: M-A, M-B and the two Lows."""

import io
import json

from textual.widgets import Input

from herdr_inbox import hook, proc, service
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp


def caller(session="sess-A", pane="w1:p1"):
    return Caller(pane_id=pane, tab_id="w1:t1", tab_label="my-tab", session_id=session, cwd="/repo")


def failed_answer(store, monkeypatch, error_env=("FAKE_HERDR_FAIL", "agent prompt")):
    qid = service.ask(store, caller(), "Merge it?")["id"]
    monkeypatch.setenv(*error_env)
    assert service.answer(store, qid, "yes").delivery_error
    monkeypatch.delenv(error_env[0])
    return qid


# -- M-A: a retry never types into a pane the operator may be using ----------------

def test_retry_skips_focused_pane(store, fake_herdr, monkeypatch):
    qid = failed_answer(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "1")
    fake_herdr.reset()
    service.retry_deliveries(store, "w1:p1")
    assert fake_herdr.calls_of("agent", "prompt") == []
    assert store.get(qid).delivered_at is None
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "0")
    service.retry_deliveries(store, "w1:p1")
    assert store.get(qid).delivered_at


def test_retry_skips_blocked_agent(store, fake_herdr, monkeypatch):
    qid = failed_answer(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "blocked")
    fake_herdr.reset()
    service.retry_deliveries(store, "w1:p1")
    assert fake_herdr.calls_of("agent", "prompt") == [] and store.get(qid).delivered_at is None


def test_first_delivery_from_the_popup_ignores_focus(store, fake_herdr, monkeypatch):
    # The operator answers from the modal popup, usually opened over the agent's own pane.
    qid = service.ask(store, caller(), "Merge it?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "1")
    assert service.answer(store, qid, "yes").delivered_at


# -- Low: stale deliveries stop being retried --------------------------------------

def test_retry_gives_up_on_a_changed_conversation(store, fake_herdr, monkeypatch):
    qid = failed_answer(store, monkeypatch, ("FAKE_HERDR_SESSION", "sess-NEW"))
    monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-NEW")
    for _ in range(30):
        service.retry_deliveries(store, "w1:p1")
    assert len(fake_herdr.calls_of("agent", "get")) <= service.MAX_STALE_RETRIES + 1
    assert [a.id for a in store.unacknowledged_answers("sess-A", "w1:p1")] == [qid]  # still for check_answers


def test_retry_keeps_trying_a_blocked_or_failing_agent(store, fake_herdr, monkeypatch):
    qid = failed_answer(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    for _ in range(service.MAX_STALE_RETRIES + 5):
        service.retry_deliveries(store, "w1:p1")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    service.retry_deliveries(store, "w1:p1")
    assert store.get(qid).delivered_at


# -- Low: the answer box says which question it answers ----------------------------

async def test_answer_box_names_its_question(store, fake_herdr):
    service.ask(store, caller(), "first?")
    service.ask(store, caller(), "second?")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("a")
        box = app.query_one(Input)
        assert box.border_title == "Answering Q2" and "Q2" in box.placeholder


# -- M-B: a claude nested inside a herdr pane's agent is not that agent ------------

TOP = {100: (50, "zsh"), 50: (40, "claude"), 40: (30, "zsh"), 30: (1, "herdr")}
NESTED = {200: (150, "uv"), 150: (140, "claude"), 140: (130, "timeout"), 130: (120, "zsh"),
          120: (50, "zsh"), 50: (40, "claude"), 40: (30, "zsh"), 30: (1, "herdr")}


def fake_tree(monkeypatch, table, start):
    monkeypatch.setattr(proc, "_parent", lambda pid: table.get(pid))
    monkeypatch.setattr(proc.os, "getppid", lambda: start)


def test_nested_detection(monkeypatch):
    fake_tree(monkeypatch, TOP, 100)
    assert not proc.nested_claude()
    fake_tree(monkeypatch, NESTED, 200)
    assert proc.nested_claude()
    # npm/npx wrappers or an unreadable tree: never claim nested
    fake_tree(monkeypatch, {100: (60, "node"), 60: (50, "claude"), 50: (40, "npx"), 40: (1, "zsh")}, 100)
    assert not proc.nested_claude()
    fake_tree(monkeypatch, {}, 100)
    assert not proc.nested_claude()


def test_nested_claude_cannot_file_into_the_pane(store, fake_herdr, monkeypatch):
    fake_tree(monkeypatch, NESTED, 200)
    c = service.resolve_caller()
    assert c.pane_id is None
    r = service.ask(store, c, "Nested question?")
    assert r["status"] == "unavailable" and "chat" in r["message"]
    assert store.list(include_closed=True) == [] and fake_herdr.calls_of("notification", "show") == []


def test_nested_claude_hooks_are_silent(fake_herdr, monkeypatch, capsys):
    fake_tree(monkeypatch, NESTED, 200)
    monkeypatch.setattr("sys.stdin", io.StringIO(json.dumps({"session_id": "child", "source": "startup"})))
    assert hook.run("session-start") == 0
    assert capsys.readouterr().out == ""
    fake_tree(monkeypatch, TOP, 100)
    monkeypatch.setattr("sys.stdin", io.StringIO(json.dumps({"session_id": "s", "source": "startup"})))
    hook.run("session-start")
    assert "<herdr-inbox>" in capsys.readouterr().out
