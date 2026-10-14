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
- Whenever you would ask the operator something in chat and can keep working meanwhile, call ask_operator instead. Make the question self-contained (the operator reads it without this chat) and put choices in `options`.
- Ask once. Never restate the question in chat and never call ask_operator again for it. Continue with work that does not depend on the answer.
- Answers arrive later as messages starting with `[inbox] Q<n> answered`; act on them then.
- After compaction or resume, call list_my_questions before asking anything new; check_answers returns answers you may have missed. If you resolve a question yourself, call withdraw_question.
- Blocking or destructive confirmations (anything you must not do without an explicit go) still go through the normal flow: ask in chat and wait.
- When you brief a subagent, pass this rule on: questions for the operator go through ask_operator, once, with `asked_by` set to its role, and it keeps working.
Details: skill herdr-inbox:operator-questions.
</herdr-inbox>"""

SUBAGENT_RULES = """<herdr-inbox>
If you need a decision from the human operator and can keep working without it, call the `ask_operator` MCP tool once with a self-contained question and `asked_by` set to your role, then continue. Do not wait for the answer and do not repeat the question in your report beyond its Q-number; the answer is delivered to the main session. Blocking or destructive confirmations go back to your caller instead.
</herdr-inbox>"""


def _pending(session_id: str) -> list[str]:
    from .store import Store  # sqlite only; still cheap

    store = Store()
    try:
        rows = store.db.execute(
            "SELECT id, status, question, answer FROM questions WHERE session_id = ?"
            " AND (status = 'open' OR (status = 'answered' AND acknowledged_at IS NULL)) ORDER BY id",
            (session_id,),
        ).fetchall()
    finally:
        store.close()
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


def run(event: str) -> int:
    if not os.environ.get("HERDR_PANE_ID"):
        return 0  # outside herdr nobody watches the inbox: say nothing, chat stays the channel
    try:
        raw = sys.stdin.read()
        payload = json.loads(raw) if raw.strip() else {}
        if event == "session-start":
            name, context = "SessionStart", session_start_context(payload)
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
    return 0
