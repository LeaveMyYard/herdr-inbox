"""Claude Code hooks: inject the inbox rules into every session.

Must be fast and must never fail the session: any error means exit 0 with no
output. Deliberately imports nothing heavy (no mcp, no textual).
"""

from __future__ import annotations

import json
import os
import sys

SESSION_RULES = """<herdr-inbox>
You have a non-blocking question inbox to the human operator: the `ask_operator` MCP tool (server herdr-inbox).
- Whenever you would ask the operator something in chat and can keep working meanwhile, call ask_operator instead. Make the question self-contained (the operator reads it without this chat). Like AskUserQuestion: choices in `options` (strings or `{label, description}`), a short `header`, `multi_select`, or up to 4 related questions at once via `questions` (then give the call a short `title`).
- Prefer it over AskUserQuestion, which stops you at a prompt the operator may not be watching.
- To flag something for the operator to check (no answer needed), call alert_operator instead of writing it in chat.
- Ask once. Never restate the question in chat and never call ask_operator again for it. Continue with work that does not depend on the answer.
- Answers arrive later as `[inbox]` messages naming the Q ids (several can come in one message); act on them then. If one seems overdue, call check_answers.
- After compaction or resume, call list_my_questions before asking anything new; check_answers returns answers you may have missed. If you resolve a question yourself, call withdraw_question.
- Blocking or destructive confirmations (anything you must not do without an explicit go) still go through the normal flow: ask in chat and wait.
- When you brief a subagent, pass this rule on: questions for the operator go through ask_operator, once, with `asked_by` set to its role, and it keeps working.
Details: skill herdr-inbox:operator-questions.
</herdr-inbox>"""

SUBAGENT_RULES = """<herdr-inbox>
If you need a decision from the human operator and can keep working without it, call the `ask_operator` MCP tool once with a self-contained question and `asked_by` set to your role, then continue. Do not wait for the answer and do not repeat the question in your report beyond its Q-number; the answer is delivered to the main session. Blocking or destructive confirmations go back to your caller instead. Something the operator should check, with no answer needed, goes through `alert_operator` (title, message, severity; `asked_by` your role).
</herdr-inbox>"""


# A busy or locked inbox DB must never cost the session its rules: the lookups below give
# up after these and the hook prints what it has.
PENDING_TIMEOUT = 0.2
RECORD_TIMEOUT = 0.5


def _pending(session_id: str) -> list[str]:
    """This session's open / unseen questions, read-only (no schema work, no migration) and
    with a short timeout; [] if the DB is missing, locked or unreadable."""
    import sqlite3

    from .store import db_path

    path = db_path()
    if not path.exists():
        return []
    try:
        db = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=PENDING_TIMEOUT)
        try:
            rows = db.execute(
                "SELECT id, status, question, answer FROM questions WHERE session_id = ?"
                " AND (status = 'open' OR (status = 'answered' AND acknowledged_at IS NULL)) ORDER BY id",
                (session_id,),
            ).fetchall()
        finally:
            db.close()
    except sqlite3.Error:
        return []
    lines = []
    for qid, status, question, answer in rows:
        q = " ".join(question.split())
        if status == "open":
            lines.append(f"Q{qid} (open): {q}")
        else:
            lines.append(f"Q{qid} (answered, not yet seen): {q} -> {answer}")
    return lines


def session_start_context(payload: dict) -> str:
    text = SESSION_RULES
    source = payload.get("source")
    session_id = payload.get("session_id")
    if source in ("compact", "resume") and session_id:
        lines = _pending(session_id)
        if lines:
            text += (
                "\nYour questions already with the operator (do not ask them again; "
                "call check_answers for answers you have not seen):\n" + "\n".join(lines)
            )
    return text


def _remember_transcript(payload: dict) -> None:
    """Record where this session's transcript is: deliveries are verified against it."""
    if payload.get("session_id") and payload.get("transcript_path"):
        from .store import Store

        with Store(timeout=RECORD_TIMEOUT) as store:
            store.record_session(payload["session_id"], payload["transcript_path"])


def run(event: str) -> int:
    try:
        from .proc import nested_claude

        if not os.environ.get("HERDR_PANE_ID") or nested_claude():
            return 0  # not the pane's own agent: nobody watches the inbox for it, chat stays the channel
    except BaseException:  # noqa: BLE001
        return 0
    try:
        raw = sys.stdin.read()
        payload = json.loads(raw) if raw.strip() else {}
        if event == "session-start":
            name = "SessionStart"
            try:
                context = session_start_context(payload)
            except BaseException:  # noqa: BLE001 - the rules alone, rather than nothing
                context = SESSION_RULES
        elif event == "subagent-start":
            name, context = "SubagentStart", SUBAGENT_RULES
        else:
            return 0
        out = json.dumps({"hookSpecificOutput": {"hookEventName": name, "additionalContext": context}})
    except BaseException:  # noqa: BLE001 - a hook must never break the session
        return 0
    try:
        sys.stdout.write(out + "\n")
        sys.stdout.flush()
    except BaseException:  # noqa: BLE001
        pass
    if event == "session-start":
        # after the rules are out: an optimisation (verification falls back to the
        # projects-dir glob), so a locked or unwritable DB costs nothing but this
        try:
            _remember_transcript(payload)
        except BaseException:  # noqa: BLE001
            pass
    return 0
