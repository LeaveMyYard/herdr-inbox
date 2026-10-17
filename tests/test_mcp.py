"""The four MCP tools, called through the real server object and over real stdio."""

import os
import sys

import pytest
from mcp import Client, StdioServerParameters

from herdr_inbox import service
from herdr_inbox.server import mcp
from herdr_inbox.store import Store


async def call(client, name, args=None):
    result = await client.call_tool(name, args or {})
    assert not result.is_error, result
    return result.structured_content


async def test_tools_are_registered_with_guidance():
    async with Client(mcp) as client:
        tools = {t.name: t for t in (await client.list_tools()).tools}
    assert set(tools) == {"ask_operator", "check_answers", "list_my_questions", "withdraw_question",
                          "alert_operator", "withdraw_alert"}
    desc = tools["ask_operator"].description
    for phrase in ("instead of asking in chat", "self-contained", "options", "names the question id (Q<n>)", "Do not repeat"):
        assert phrase in desc
    assert set(tools["ask_operator"].input_schema["properties"]) == {"question", "options", "context", "asked_by", "header", "multi_select", "questions", "title"}


async def test_full_round_trip_through_registered_tools(migrated_db, fake_herdr, monkeypatch):
    async with Client(mcp) as client:
        r = await call(client, "ask_operator", {"question": "Deploy to prod?", "options": ["yes", "no"],
                                                "asked_by": "release agent"})
        assert r["status"] == "open" and r["id"] == 1
        assert "Recorded as Q1" in r["message"]

        dup = await call(client, "ask_operator", {"question": "deploy to PROD"})
        assert dup["status"] == "already_open" and "Do not ask again" in dup["message"]

        mine = await call(client, "list_my_questions")
        assert mine["questions"] == [{"id": 1, "status": "open", "question": "Deploy to prod?"}]
        assert (await call(client, "check_answers"))["answers"] == []

        # operator answers from the TUI/CLI side; delivery fails (agent at a dialog),
        # so the answer waits for check_answers
        monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
        store = Store()
        service.answer(store, 1, "no, wait for Monday")
        store.close()
        monkeypatch.delenv("FAKE_HERDR_FAIL")

        got = await call(client, "check_answers")
        assert got["answers"] == [{"id": 1, "question": "Deploy to prod?", "answer": "no, wait for Monday"}]
        assert (await call(client, "check_answers"))["answers"] == []  # acknowledged

        again = await call(client, "ask_operator", {"question": "Deploy to prod?"})
        assert again["status"] == "answered" and again["answer"] == "no, wait for Monday"

    # identity came from herdr, per call
    assert ["pane", "get", "w1:p1"] in fake_herdr.calls()
    assert ["tab", "get", "w1:t1"] in fake_herdr.calls()
    assert Store().get(1).session_id == "aaaaaaaa-0000-4000-8000-00000000000a"


async def test_withdraw_question(migrated_db, fake_herdr):
    async with Client(mcp) as client:
        await call(client, "ask_operator", {"question": "Which bucket?"})
        r = await call(client, "withdraw_question", {"id": 1, "reason": "found it in terraform"})
        assert r["status"] == "withdrawn"
        assert (await call(client, "list_my_questions"))["questions"] == []
        closed = await call(client, "list_my_questions", {"include_closed": True})
        assert closed["questions"][0]["status"] == "withdrawn"
        r = await call(client, "withdraw_question", {"id": 1, "reason": "again"})
        assert r["status"] == "error"
    assert Store().list() == []  # gone from the operator's view
    assert fake_herdr.calls_of("pane", "report-metadata")[-1][-2:] == ["--clear-token", "inbox_q"]


async def test_cannot_withdraw_another_sessions_question(migrated_db, fake_herdr, monkeypatch):
    async with Client(mcp) as client:
        await call(client, "ask_operator", {"question": "Which bucket?"})
        monkeypatch.setenv("FAKE_HERDR_SESSION", "bbbbbbbb-0000-4000-8000-00000000000b")  # e.g. after /clear
        r = await call(client, "withdraw_question", {"id": 1, "reason": "x"})
        assert r["status"] == "error"
        assert (await call(client, "list_my_questions"))["questions"] == []


async def test_session_change_is_resolved_per_call(migrated_db, fake_herdr, monkeypatch):
    async with Client(mcp) as client:
        await call(client, "ask_operator", {"question": "Which bucket?"})
        monkeypatch.setenv("FAKE_HERDR_SESSION", "bbbbbbbb-0000-4000-8000-00000000000b")
        r = await call(client, "ask_operator", {"question": "Which bucket?"})
        assert r["status"] == "open" and r["id"] == 2  # new conversation, not a repeat


async def test_claude_session_env_is_the_fallback(migrated_db, fake_herdr, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_SESSION", "")  # herdr knows the pane but not the session
    monkeypatch.setenv("CLAUDE_CODE_SESSION_ID", "claude-sess")
    async with Client(mcp) as client:
        r = await call(client, "ask_operator", {"question": "Which session?"})
    assert Store().get(r["id"]).session_id == "claude-sess"


@pytest.mark.parametrize("mode", ["legacy", "auto"])
async def test_stdio_server_end_to_end(migrated_db, fake_herdr, mode):
    """Spawn `inbox mcp` as a real subprocess and talk MCP over its stdin/stdout."""
    env = {k: os.environ[k] for k in ("HERDR_BIN_PATH", "FAKE_HERDR_LOG", "HERDR_INBOX_DB", "HERDR_PANE_ID", "PATH")}
    params = StdioServerParameters(command=sys.executable, args=["-m", "herdr_inbox.cli", "mcp"], env=env)
    async with Client(params, mode=mode, read_timeout_seconds=20) as client:
        r = await call(client, "ask_operator", {"question": "Over stdio?", "context": "e2e test"})
        assert r == {
            "id": 1,
            "status": "open",
            "message": "Recorded as Q1. The operator will answer asynchronously; the answer will arrive in this "
            "conversation as a message starting with `[inbox]` that names Q1. Continue with work that does not "
            "depend on it. Do not repeat the question in chat or call ask_operator for it again. "
            "If an answer seems overdue, call check_answers.",
        }
        mine = await call(client, "list_my_questions")
        assert mine["questions"][0]["question"] == "Over stdio?"
    q = Store().get(1)
    assert (q.pane_id, q.tab_label, q.session_id, q.context) == ("w1:p1", "my-tab", "aaaaaaaa-0000-4000-8000-00000000000a", "e2e test")
    assert fake_herdr.calls_of("notification", "show")


# -- P0b: MCP tools never migrate (store.py, MIGRATION_REVISION) ------------------------------

from herdr_inbox import server  # noqa: E402

from .compat_prebridge import (  # noqa: E402
    CURRENT_PRE_BRIDGE, columns, make_older_head, schema_statements, set_stamp, stamp_of, traced)


async def test_tools_on_a_pre_bridge_stamp_work_without_any_schema_work(migrated_db, fake_herdr, monkeypatch):
    path = Store().path
    set_stamp(path, CURRENT_PRE_BRIDGE)
    statements = traced(monkeypatch)
    async with Client(mcp) as client:
        assert (await call(client, "ask_operator", {"question": "Stale stamp?"}))["status"] == "open"
        assert (await call(client, "list_my_questions"))["questions"][0]["question"] == "Stale stamp?"
        assert (await call(client, "alert_operator", {"title": "Look"}))["status"] == "open"
    assert stamp_of(path) == CURRENT_PRE_BRIDGE
    assert statements and schema_statements(statements) == []


async def test_tools_on_an_older_schema_return_the_unavailable_receipt(migrated_db, fake_herdr, monkeypatch):
    path = Store().path
    make_older_head(path)
    monkeypatch.setattr(server, "BUSY_BACKOFF", (0.0, 0.0, 0.0))
    started = []
    monkeypatch.setattr(server, "_ensure_worker", lambda: started.append(1))
    async with Client(mcp) as client:
        for name, args in (("ask_operator", {"question": "Too early?"}), ("check_answers", {}),
                           ("list_my_questions", {}), ("withdraw_question", {"id": 1, "reason": "x"}),
                           ("alert_operator", {"title": "Look"}), ("withdraw_alert", {"id": 1})):
            assert await call(client, name, args) == service.NOT_MIGRATED, name
    assert started == [1] * 6  # once per call: the worker migrates
    assert stamp_of(path) == CURRENT_PRE_BRIDGE
    assert "sending_at" not in columns(path, "remote_outbox")
    assert Store(path, migrate=True).list(include_closed=True) == []  # nothing was filed


async def test_a_tool_waits_briefly_for_the_worker_it_started(migrated_db, fake_herdr, monkeypatch):
    """A fresh install: the first ask_operator can come before the worker's first tick."""
    path = Store().path
    make_older_head(path)
    monkeypatch.setattr(server, "BUSY_BACKOFF", (0.0, 0.0, 0.0))
    monkeypatch.setattr(server, "_ensure_worker", lambda: Store(path, migrate=True).close())
    async with Client(mcp) as client:
        assert (await call(client, "ask_operator", {"question": "Just in time?"}))["status"] == "open"
