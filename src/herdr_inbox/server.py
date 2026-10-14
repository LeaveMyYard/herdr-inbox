"""MCP stdio server: the agent-facing side of the inbox.

mcp 2.x renamed FastMCP to ``MCPServer``; the decorator API is the same.
"""

from __future__ import annotations

import logging
from typing import Any

from mcp.server.mcpserver import MCPServer

from . import service
from .store import Store

INSTRUCTIONS = (
    "Non-blocking question inbox for the human operator. Use ask_operator instead of asking the "
    "operator in chat whenever you can keep working. Ask once; the answer arrives later as a "
    "message starting with `[inbox] Q<n> answered`. After compaction call list_my_questions "
    "before asking anything."
)

ASK_DESCRIPTION = """Ask the human operator a question WITHOUT blocking. Returns immediately with a receipt.

Use this instead of asking in chat whenever work can continue without the answer.
The operator answers asynchronously; the answer will arrive in this conversation as a
message starting with `[inbox] Q<n>`. After calling it, continue with work that does not
depend on the answer. Do not repeat the question in chat and do not call ask_operator for
it again: a repeat is detected and only returns the existing receipt.

Keep the question self-contained: the operator reads it without your chat history, so name
the repo/PR/file and what you need decided. Put discrete choices in `options` (the operator
can pick one with a single key). `context` holds supporting detail; `asked_by` describes who
is asking (e.g. "reviewer PR 595") when you are a subagent or one of several roles."""

mcp = MCPServer("herdr-inbox", instructions=INSTRUCTIONS)


@mcp.tool(description=ASK_DESCRIPTION)
def ask_operator(
    question: str,
    options: list[str] | None = None,
    context: str | None = None,
    asked_by: str | None = None,
) -> dict[str, Any]:
    with Store() as store:
        return service.ask(store, service.resolve_caller(), question, options, context, asked_by)


@mcp.tool()
def check_answers() -> dict[str, Any]:
    """Return operator answers to your questions that you have not seen yet, and mark them seen.

    Answers are normally delivered as `[inbox] Q<n> answered` messages; use this as a fallback
    (e.g. after compaction or if a delivery was missed)."""
    caller = service.resolve_caller()
    with Store() as store:
        answers = store.unacknowledged_answers(caller.session_id, caller.pane_id)
        store.acknowledge([q.id for q in answers])
    return {"answers": [{"id": q.id, "question": q.question, "answer": q.answer} for q in answers]}


@mcp.tool()
def list_my_questions(include_closed: bool = False) -> dict[str, Any]:
    """List the questions this session has asked the operator, with their status and answers.

    Call this after compaction or resume BEFORE asking anything, so you do not re-ask a
    question that is already open or answered."""
    caller = service.resolve_caller()
    with Store() as store:
        questions = store.mine(caller.session_id, caller.pane_id, include_closed)
    return {"questions": [q.brief() for q in questions]}


@mcp.tool()
def withdraw_question(id: int, reason: str) -> dict[str, Any]:
    """Withdraw one of your open questions because you resolved it yourself.

    It is removed from the operator's inbox."""
    caller = service.resolve_caller()
    with Store() as store:
        mine = {q.id: q for q in store.mine(caller.session_id, caller.pane_id, include_closed=True)}
        if id not in mine:
            return {"id": id, "status": "error", "message": f"Q{id} is not a question of this session."}
        if not service.close(store, id, "withdrawn", reason):
            return {"id": id, "status": "error", "message": f"Q{id} is {mine[id].status}, not open."}
    return {"id": id, "status": "withdrawn", "message": f"Q{id} withdrawn."}


def run() -> None:
    # stdout is the MCP channel; logs go to stderr.
    logging.basicConfig(level=logging.WARNING, format="herdr-inbox: %(message)s")
    mcp.run("stdio")
