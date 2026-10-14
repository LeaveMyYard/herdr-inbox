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
    assert set(tools) == {"ask_operator", "check_answers", "list_my_questions", "withdraw_question"}
    desc = tools["ask_operator"].description
    for phrase in ("instead of asking in chat", "self-contained", "options", "names the question id (Q<n>)", "Do not repeat"):
        assert phrase in desc
    assert set(tools["ask_operator"].input_schema["properties"]) == {"question", "options", "context", "asked_by", "header", "multi_select", "questions", "title"}


async def test_full_round_trip_through_registered_tools(fake_herdr, monkeypatch):
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
    assert Store().get(1).session_id == "sess-A"


async def test_withdraw_question(fake_herdr):
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


async def test_cannot_withdraw_another_sessions_question(fake_herdr, monkeypatch):
    async with Client(mcp) as client:
        await call(client, "ask_operator", {"question": "Which bucket?"})
        monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-B")  # e.g. after /clear
        r = await call(client, "withdraw_question", {"id": 1, "reason": "x"})
        assert r["status"] == "error"
        assert (await call(client, "list_my_questions"))["questions"] == []


async def test_session_change_is_resolved_per_call(fake_herdr, monkeypatch):
    async with Client(mcp) as client:
        await call(client, "ask_operator", {"question": "Which bucket?"})
        monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-B")
        r = await call(client, "ask_operator", {"question": "Which bucket?"})
        assert r["status"] == "open" and r["id"] == 2  # new conversation, not a repeat


async def test_claude_session_env_is_the_fallback(fake_herdr, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_SESSION", "")  # herdr knows the pane but not the session
    monkeypatch.setenv("CLAUDE_CODE_SESSION_ID", "claude-sess")
    async with Client(mcp) as client:
        r = await call(client, "ask_operator", {"question": "Which session?"})
    assert Store().get(r["id"]).session_id == "claude-sess"


@pytest.mark.parametrize("mode", ["legacy", "auto"])
async def test_stdio_server_end_to_end(fake_herdr, mode):
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
            "depend on it. Do not repeat the question in chat or call ask_operator for it again.",
        }
        mine = await call(client, "list_my_questions")
        assert mine["questions"][0]["question"] == "Over stdio?"
    q = Store().get(1)
    assert (q.pane_id, q.tab_label, q.session_id, q.context) == ("w1:p1", "my-tab", "sess-A", "e2e test")
    assert fake_herdr.calls_of("notification", "show")
