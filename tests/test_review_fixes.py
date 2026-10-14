"""Regression tests for the PR #1 review findings H1, M1, M2, H2, L1-L4."""

import json
import sqlite3
import threading
import time

from mcp import Client

from herdr_inbox import service
from herdr_inbox.server import mcp
from herdr_inbox.service import Caller
from herdr_inbox.store import Store

from .test_hook import context_of, run_hook


def caller(session="sess-A", pane="w1:p1"):
    return Caller(pane_id=pane, tab_id="w1:t1", tab_label="my-tab", session_id=session, cwd="/repo")


# -- H1: deliver only into the conversation that asked ----------------------------

def test_no_delivery_when_pane_holds_another_conversation(store, fake_herdr, monkeypatch):
    qid = service.ask(store, caller(), "Merge it?")["id"]
    monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-after-clear")
    q = service.answer(store, qid, "yes, merge")
    assert fake_herdr.calls_of("agent", "prompt") == []
    assert q.delivered_at is None and "conversation changed" in q.delivery_error
    assert [a.id for a in store.unacknowledged_answers("sess-A", "w1:p1")] == [qid]


def test_no_delivery_into_a_different_agent(store, fake_herdr, monkeypatch):
    qid = service.ask(store, caller(), "Merge it?")["id"]
    monkeypatch.setenv("FAKE_HERDR_AGENT_KIND", "codex")
    monkeypatch.setenv("FAKE_HERDR_SESSION", "codex-thread")
    q = service.answer(store, qid, "yes")
    assert fake_herdr.calls_of("agent", "prompt") == [] and q.delivery_error


def test_question_without_session_falls_back_to_live_agent(store, fake_herdr):
    qid = service.ask(store, caller(session=None), "Merge it?")["id"]
    assert service.answer(store, qid, "yes").delivered_at


# -- M1: a delivered answer counts as seen ---------------------------------------

def test_delivered_answer_is_acknowledged(store, fake_herdr):
    qid = service.ask(store, caller(), "Merge it?")["id"]
    q = service.answer(store, qid, "yes")
    assert q.delivered_at and q.acknowledged_at
    assert store.unacknowledged_answers("sess-A", "w1:p1") == []
    ctx = context_of(run_hook("session-start", {"session_id": "sess-A", "source": "compact"}))
    assert f"Q{qid}" not in ctx


# -- M2: failed deliveries are retried by the asking session's MCP server ---------

def test_retry_delivers_after_a_failed_attempt(store, fake_herdr, monkeypatch):
    qid = service.ask(store, caller(), "Merge it?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")  # e.g. agent_blocked at a dialog
    assert service.answer(store, qid, "yes").delivery_error
    service.retry_deliveries(store, "w1:p1")  # still blocked: stays pending
    assert store.get(qid).delivered_at is None
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    service.retry_deliveries(store, "w1:p1")
    q = store.get(qid)
    assert q.delivered_at and q.acknowledged_at and q.delivery_error is None
    assert len(fake_herdr.calls_of("agent", "prompt")) == 3
    service.retry_deliveries(store, "w1:p1")  # nothing left to do
    assert len(fake_herdr.calls_of("agent", "prompt")) == 3


def test_retry_skips_seen_and_other_panes(store, fake_herdr, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    seen = service.ask(store, caller(), "one?")["id"]
    other = service.ask(store, caller(pane="w1:p2"), "two?")["id"]
    service.answer(store, seen, "a")
    service.answer(store, other, "b")
    store.acknowledge([seen])  # agent got it through check_answers
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    fake_herdr.reset()
    service.retry_deliveries(store, "w1:p1")
    assert fake_herdr.calls_of("agent", "prompt") == []


def test_server_retry_loop_runs_in_background(store, fake_herdr, monkeypatch):
    from herdr_inbox import server

    qid = service.ask(store, caller(), "Merge it?")["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, qid, "yes")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    stop = threading.Event()
    server.start_retry_loop(interval=0.05, stop=stop)
    try:
        deadline = time.time() + 5
        while store.get(qid).delivered_at is None and time.time() < deadline:
            time.sleep(0.05)
    finally:
        stop.set()
    assert store.get(qid).delivered_at


# -- H2: outside herdr nothing is filed into an unwatched inbox ------------------

async def test_ask_outside_herdr_says_ask_in_chat(fake_herdr, monkeypatch):
    monkeypatch.delenv("HERDR_PANE_ID")
    async with Client(mcp) as client:
        r = (await client.call_tool("ask_operator", {"question": "Outside herdr?"})).structured_content
        assert r["status"] == "unavailable" and "Ask the operator in chat" in r["message"]
        assert (await client.call_tool("check_answers", {})).structured_content == {"answers": []}
    assert Store().list(include_closed=True) == []
    assert fake_herdr.calls() == []


def test_hooks_are_silent_outside_herdr(fake_herdr, monkeypatch):
    monkeypatch.delenv("HERDR_PANE_ID")
    for event, payload in (("session-start", {"session_id": "s", "source": "startup"}), ("subagent-start", {})):
        proc = run_hook(event, payload)
        assert (proc.returncode, proc.stdout) == (0, "")


# -- L1: resume lists only this session's questions -----------------------------

def test_resume_does_not_list_other_conversations_in_the_pane(store, fake_herdr):
    service.ask(store, caller(session="old-conversation"), "Old question?")
    service.ask(store, caller(session="sess-A"), "My question?")
    ctx = context_of(run_hook("session-start", {"session_id": "sess-A", "source": "resume"}))
    assert "My question?" in ctx and "Old question" not in ctx


# -- L2: dedupe + insert is atomic ------------------------------------------------

def test_concurrent_identical_asks_create_one_question(store, fake_herdr, monkeypatch):
    real_find = Store.find_duplicate
    in_window = threading.Event()

    def slow_find(self, *args):
        found = real_find(self, *args)
        if threading.current_thread().name == "first":
            in_window.set()
            time.sleep(0.4)  # the other ask runs while this one sits between find and insert
        return found

    monkeypatch.setattr(Store, "find_duplicate", slow_find)
    results = {}

    def ask(name):
        with Store() as s:
            results[name] = service.ask(s, caller(), "Merge PR 7?")

    first = threading.Thread(target=ask, args=("first",), name="first")
    first.start()
    in_window.wait(2)
    second = threading.Thread(target=ask, args=("second",), name="second")
    second.start()
    first.join(5)
    second.join(5)
    assert sorted(r["status"] for r in results.values()) == ["already_open", "open"]
    assert len(store.list()) == 1


# -- L3: an old answer is not handed out forever ---------------------------------

def test_answered_dedupe_is_time_bounded(store, fake_herdr):
    qid = service.ask(store, caller(), "Proceed with the next step?")["id"]
    service.answer(store, qid, "yes")
    assert service.ask(store, caller(), "Proceed with the next step?")["status"] == "answered"
    store.db.execute("UPDATE questions SET answered_at = '2020-01-01T00:00:00+00:00' WHERE id = ?", (qid,))
    again = service.ask(store, caller(), "Proceed with the next step?")
    assert again["status"] == "open" and again["id"] != qid


# -- L4: forked sessions get the rules too ---------------------------------------

def test_matcher_includes_fork(fake_herdr):
    from pathlib import Path

    hooks = json.loads((Path(__file__).resolve().parents[1] / "hooks/hooks.json").read_text())["hooks"]
    assert "fork" in hooks["SessionStart"][0]["matcher"].split("|")
    ctx = context_of(run_hook("session-start", {"session_id": "forked", "source": "fork"}))
    assert ctx.startswith("<herdr-inbox>")
