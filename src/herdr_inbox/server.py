"""MCP stdio server: the agent-facing side of the inbox.

mcp 2.x renamed FastMCP to ``MCPServer``; the decorator API is the same.
"""

from __future__ import annotations

import functools
import logging
import os
import sqlite3
import threading
import time
from typing import Any

from typing_extensions import NotRequired, TypedDict  # pydantic needs this TypedDict on 3.11

from mcp.server.mcpserver import MCPServer

from . import service
from .store import Store

INSTRUCTIONS = (
    "Non-blocking question inbox for the human operator. Use ask_operator instead of asking the "
    "operator in chat whenever you can keep working. Ask once; the answer arrives later as a "
    "`[inbox]` message naming the Q id. After compaction call list_my_questions "
    "before asking anything."
)

ASK_DESCRIPTION = """Ask the human operator a question WITHOUT blocking. Returns immediately with a receipt.

Use this instead of asking in chat whenever work can continue without the answer.
The operator answers asynchronously; the answer will arrive in this conversation as a
message starting with `[inbox]` that names the question id (Q<n>). After calling it, continue
with work that does not depend on the answer. Do not repeat the question in chat and do not
call ask_operator for it again: a repeat is detected and only returns the existing receipt.

Keep the question self-contained: the operator reads it without your chat history, so name
the repo/PR/file and what you need decided. The operator answers in a picker like
AskUserQuestion's:
- `options`: up to 9 choices, each a plain string or `{"label", "description"}` (label short,
  description says what picking it means). The operator can always type a free answer instead
  and attach a note to a choice.
- `header`: a short chip shown above the question (max 12 chars), e.g. "DB choice".
- `multi_select`: true when several options may be picked together.
- `preview` (per option, optional): text shown beside the options while that option is
  highlighted (a code snippet, a config diff, an ASCII mock); monospace, no markup.
- `title`: a short name for the whole call (≤60 chars), e.g. "Worker rollout settings". The
  operator's inbox lists calls by this name, so ALWAYS set it when you pass `questions`;
  optional for a single question (its header or text is used instead).
- `questions`: instead of `question`, up to 4 related questions in one call, each
  `{"question", "header"?, "options"?, "multi_select"?}`. The operator answers them together;
  each gets its own Q id.
- `context`: supporting detail; `asked_by`: who is asking (e.g. "reviewer PR 42") when you are
  a subagent or one of several roles.
Each answer is one line: the picked label(s), notes in parentheses, or the free text. Answers
the operator submits together arrive in one `[inbox]` message."""


class Option(TypedDict):
    label: str
    description: NotRequired[str]
    preview: NotRequired[str]


class QuestionItem(TypedDict):
    question: str
    header: NotRequired[str]
    options: NotRequired[list[str | Option]]
    multi_select: NotRequired[bool]


BUSY_BACKOFF = (0.3, 1.0, 2.0)
TOOL_BUSY_TIMEOUT = 3.0  # per attempt; busy_safe retries, so a tool never hangs for minutes


def tool_store() -> Store:
    return Store(timeout=TOOL_BUSY_TIMEOUT)


def busy_safe(fn):
    """Retry a tool when SQLite reports the database locked; never surface a raw error."""

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        for delay in (*BUSY_BACKOFF, None):
            try:
                return fn(*args, **kwargs)
            except sqlite3.OperationalError as exc:
                if "locked" not in str(exc) and "busy" not in str(exc):
                    raise
                if delay is None:
                    return {"status": "error", "message": "The inbox database is busy right now; "
                            "call this tool again in a few seconds."}
                time.sleep(delay)

    return wrapper


mcp = MCPServer("herdr-inbox", instructions=INSTRUCTIONS)


@mcp.tool(description=ASK_DESCRIPTION)
@busy_safe
def ask_operator(
    question: str | None = None,
    options: list[str | Option] | None = None,
    context: str | None = None,
    asked_by: str | None = None,
    header: str | None = None,
    multi_select: bool = False,
    questions: list[QuestionItem] | None = None,
    title: str | None = None,
) -> dict[str, Any]:
    if bool(question) == bool(questions):
        return {"status": "error", "message": "Pass either `question` or `questions` (not both)."}
    items = list(questions or [{"question": question, "options": options, "header": header,
                                "multi_select": multi_select}])
    if len(items) > service.MAX_QUESTIONS:
        return {"status": "error", "message": f"At most {service.MAX_QUESTIONS} questions per call."}
    if any(len(i.get("options") or []) > service.MAX_OPTIONS for i in items):
        return {"status": "error", "message": f"At most {service.MAX_OPTIONS} options; ask fewer or split the question."}
    with tool_store() as store:
        caller = service.resolve_caller()
        if questions:
            return service.ask_group(store, caller, items, context, asked_by, title=title)
        return service.ask(store, caller, question, options, context, asked_by,
                           header=header, multi_select=multi_select, title=title)


@mcp.tool()
@busy_safe
def check_answers() -> dict[str, Any]:
    """Return operator answers to your questions that you have not seen yet, and mark them seen.

    Answers are normally delivered as `[inbox]` messages naming the Q ids; use this as a fallback
    (e.g. after compaction or if a delivery was missed)."""
    caller = service.resolve_caller()
    with tool_store() as store:
        answers = store.unacknowledged_answers(caller.session_id, caller.pane_id)
        store.acknowledge([q.id for q in answers])
    return {"answers": [{"id": q.id, "question": q.question, "answer": q.answer} for q in answers]}


@mcp.tool()
@busy_safe
def list_my_questions(include_closed: bool = False) -> dict[str, Any]:
    """List the questions this session has asked the operator, with their status and answers.

    Call this after compaction or resume BEFORE asking anything, so you do not re-ask a
    question that is already open or answered."""
    caller = service.resolve_caller()
    with tool_store() as store:
        questions = store.mine(caller.session_id, caller.pane_id, include_closed)
    return {"questions": [q.brief() for q in questions]}


@mcp.tool()
@busy_safe
def withdraw_question(id: int, reason: str) -> dict[str, Any]:
    """Withdraw one of your open questions because you resolved it yourself.

    It is removed from the operator's inbox."""
    caller = service.resolve_caller()
    with tool_store() as store:
        mine = {q.id: q for q in store.mine(caller.session_id, caller.pane_id, include_closed=True)}
        if id not in mine:
            return {"id": id, "status": "error", "message": f"Q{id} is not a question of this session."}
        if not service.close(store, id, "withdrawn", reason):
            return {"id": id, "status": "error", "message": f"Q{id} is {mine[id].status}, not open."}
    return {"id": id, "status": "withdrawn", "message": f"Q{id} withdrawn."}


RETRY_SECONDS = 10.0  # also how soon an answer pending on a busy agent goes out once it is idle
log = logging.getLogger("herdr_inbox")


def start_retry_loop(interval: float = RETRY_SECONDS, stop: threading.Event | None = None) -> threading.Event:
    """Retry failed answer deliveries into this server's pane, every ``interval`` seconds.

    The asking session's own MCP server is the one process that lives as long as the
    conversation, so it owns the retry; ``deliver`` still checks the session matches.
    """
    stop = stop or threading.Event()
    pane_id = os.environ.get("HERDR_PANE_ID")

    def loop() -> None:
        while pane_id and not stop.wait(interval):
            try:
                with tool_store() as store:
                    service.retry_deliveries(store, pane_id)
            except Exception:  # noqa: BLE001 - keep retrying, never kill the server
                log.exception("delivery retry failed")

    threading.Thread(target=loop, name="inbox-retry", daemon=True).start()
    return stop


def run() -> None:
    # stdout is the MCP channel; logs go to stderr.
    logging.basicConfig(level=logging.WARNING, format="herdr-inbox: %(message)s")
    start_retry_loop()
    from . import venvs

    venvs.start_cleanup()  # old unused venvs (venvs.py); the lock was taken in cli
    mcp.run("stdio")
