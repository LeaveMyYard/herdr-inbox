"""Inbox operations: store changes plus their herdr side effects.

Used by the MCP server, the TUI and the CLI alike. herdr failures never
propagate; they are logged in ``herdr.call`` and, for answer delivery,
recorded on the question.
"""

from __future__ import annotations

import logging
import os
import uuid
from dataclasses import dataclass

from . import herdr
from .store import Question, Store, normalize

log = logging.getLogger("herdr_inbox")

# Identity for a server that has neither herdr nor a Claude session id in its
# environment: questions stay retrievable through check_answers in this process.
_PROCESS_SESSION = f"local-{uuid.uuid4().hex[:12]}"


@dataclass
class Caller:
    pane_id: str | None
    tab_id: str | None
    tab_label: str | None
    session_id: str | None
    cwd: str | None


def resolve_caller() -> Caller:
    """Who is asking. Resolved per call: /clear changes the session, panes move."""
    pane_id = os.environ.get("HERDR_PANE_ID") or None
    ident = herdr.pane_identity(pane_id) if pane_id else {}
    session_id = (
        ident.get("session_id")
        or os.environ.get("CLAUDE_CODE_SESSION_ID")
        or (None if pane_id else _PROCESS_SESSION)
    )
    return Caller(
        pane_id=pane_id,
        tab_id=ident.get("tab_id") or os.environ.get("HERDR_TAB_ID") or None,
        tab_label=ident.get("tab_label"),
        session_id=session_id,
        cwd=os.getcwd(),
    )


def first_line(text: str, width: int) -> str:
    line = text.strip().splitlines()[0] if text.strip() else ""
    return line if len(line) <= width else line[: width - 1] + "…"


def ask(
    store: Store,
    caller: Caller,
    question: str,
    options: list[str] | None = None,
    context: str | None = None,
    asked_by: str | None = None,
) -> dict:
    question = question.strip()
    dup = store.find_duplicate(normalize(question), caller.session_id, caller.pane_id)
    if dup and dup.status == "open":
        store.bump_repeat(dup.id)
        return {
            "id": dup.id,
            "status": "already_open",
            "message": f"{dup.ref} is already with the operator. Do not ask again; keep working.",
        }
    if dup and dup.status == "answered":
        store.acknowledge([dup.id])
        return {
            "id": dup.id,
            "status": "answered",
            "answer": dup.answer,
            "message": f"{dup.ref} was already answered by the operator: {dup.answer}",
        }

    q = store.insert(
        question,
        options=options or None,
        context=context,
        asked_by=asked_by,
        pane_id=caller.pane_id,
        tab_id=caller.tab_id,
        tab_label=caller.tab_label,
        session_id=caller.session_id,
        cwd=caller.cwd,
    )
    herdr.notify(f"{q.ref} · {q.tab_label or q.pane_id or 'agent'}", first_line(question, 200))
    update_token(store, q.pane_id)
    return {
        "id": q.id,
        "status": "open",
        "message": (
            f"Recorded as {q.ref}. The operator will answer asynchronously; the answer will arrive in "
            f"this conversation as a message starting with `[inbox] {q.ref}`. Continue with work that "
            f"does not depend on it. Do not repeat the question in chat or call ask_operator for it again."
        ),
    }


def delivery_text(q: Question) -> str:
    return f"[inbox] {q.ref} answered — {first_line(q.question, 80)}: {q.answer}"


def answer(store: Store, qid: int, text: str) -> Question | None:
    """Answer an open question and deliver it into the asking pane. None if not open."""
    if not store.set_answer(qid, text.strip()):
        return None
    q = store.get(qid)
    deliver(store, q)
    update_token(store, q.pane_id)
    return store.get(qid)


def deliver(store: Store, q: Question) -> None:
    if not q.pane_id:
        store.record_delivery(q.id, "no pane: answer available through check_answers")
        return
    if not herdr.agent_live(q.pane_id):
        store.record_delivery(q.id, f"no live agent in pane {q.pane_id}")
        return
    store.record_delivery(q.id, herdr.prompt_agent(q.pane_id, delivery_text(q)))


def close(store: Store, qid: int, status: str, note: str | None = None) -> bool:
    q = store.get(qid)
    if not q or not store.set_status(qid, status, note):
        return False
    update_token(store, q.pane_id)
    return True


def update_token(store: Store, pane_id: str | None) -> None:
    if pane_id:
        herdr.set_open_count(pane_id, store.open_counts().get(pane_id, 0))


def sync_tokens(store: Store) -> None:
    """Recompute ``inbox_q`` on every live pane (herdr drops tokens on restart)."""
    counts = store.open_counts()
    panes = herdr.list_pane_ids()
    if panes is None:  # herdr unreachable: at least try the panes we know about
        panes = store.known_panes()
    for pane in panes:
        herdr.set_open_count(pane, counts.get(pane, 0))
