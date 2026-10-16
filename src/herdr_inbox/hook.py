"""Claude Code hooks: inject the inbox rules into every session, and (0.9.0) hand answers
to a busy session as hook context at its next tool batch or turn end (``deliver``).

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
    """Record where this session's transcript is: deliveries are verified against it. (Not
    that it can take hook deliveries: only the delivery hook firing says so, see _touch.)"""
    if payload.get("session_id") and payload.get("transcript_path"):
        from .store import Store

        with Store(timeout=RECORD_TIMEOUT, maintenance=False) as store:
            store.record_session(payload["session_id"], payload["transcript_path"])


LIVENESS_EVERY = 60  # seconds between the delivery hook's liveness writes per session


def _touch(session_id: str, column: str = "hook_seen_at") -> None:
    """The delivery hook fired: stamp ``sessions.hook_seen_at``, at most once a minute (the
    worker leaves a WORKING session to its hooks only while this is recent)."""
    import sqlite3
    from datetime import datetime, timedelta, timezone

    from .store import db_path

    path = db_path()
    if not path.exists():
        return
    stamp = datetime.now(timezone.utc)
    cutoff = (stamp - timedelta(seconds=LIVENESS_EVERY)).isoformat(timespec="seconds")
    try:  # the common case is a read-only look: the stamp is under a minute old
        ro = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=PENDING_TIMEOUT)
        try:
            assert column in ("hook_seen_at", "watch_seen_at")
            row = ro.execute(f"SELECT {column} FROM sessions WHERE session_id = ?", (session_id,)).fetchone()
        finally:
            ro.close()
        if row and row[0] and row[0] >= cutoff:
            return  # fresh enough: no write, no writable connection
        db = sqlite3.connect(str(path), timeout=PENDING_TIMEOUT)
        try:
            with db:
                db.execute(f"INSERT INTO sessions (session_id, {column}) VALUES (?, ?) ON CONFLICT(session_id)"
                           f" DO UPDATE SET {column} = excluded.{column}",
                           (session_id, stamp.isoformat(timespec="seconds")))
        finally:
            db.close()
    except sqlite3.Error:
        pass  # locked or not migrated yet: the next firing tries again


DELIVERY_EVENTS = {"post-tool-batch": "PostToolBatch", "stop": "Stop"}


def _has_candidates(session_id: str) -> bool:
    """The fast path, read-only: any answer for this session not yet delivered or seen?"""
    import sqlite3

    from .store import db_path

    path = db_path()
    if not path.exists():
        return False
    try:
        db = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=PENDING_TIMEOUT)
        try:
            from .store import MAX_HOOK_MISSES  # stdlib-light: not .service

            # the same filter as Store.hook_candidates: rows at the miss cap or whose agent is
            # gone never open the writable store (review #26 L2)
            # this session and those it continued (sessions.continued_in, Claude Code continuation)
            return db.execute(
                "WITH RECURSIVE fam(s) AS (SELECT ? UNION SELECT sessions.session_id FROM sessions JOIN fam"
                " ON sessions.continued_in = fam.s)"
                " SELECT 1 FROM questions WHERE session_id IN (SELECT s FROM fam) AND status = 'answered'"
                " AND delivered_at IS NULL AND acknowledged_at IS NULL AND id NOT IN (SELECT id FROM question_ext"
                " WHERE COALESCE(hook_misses, 0) >= ? OR agent_gone_at IS NOT NULL) LIMIT 1",
                (session_id, MAX_HOOK_MISSES)).fetchone() is not None
        finally:
            db.close()
    except sqlite3.Error:
        return False


def deliver(payload: dict, liveness: bool = True) -> str | None:
    """This session's waiting answers as the hook's additionalContext, or None.

    Same text as typing (``[inbox] Q7 answered — …``), one message per firing. Each row is
    claimed (CAS) and recorded as hook-sent BEFORE the text is printed; the delivery worker
    verifies it in the transcript, or puts it back to pending (DESIGN.md, "Hook delivery")."""
    session = payload.get("session_id")
    if not session or payload.get("agent_id"):
        return None  # a subagent's or teammate's hook: the answer belongs to the session that asked
    if liveness:  # the PostToolBatch/Stop hooks only; the rewake watcher stamps its own column
        _touch(session)
    if not _has_candidates(session):
        return None
    from . import herdr, service, transcript
    from .store import Store

    with Store(timeout=RECORD_TIMEOUT, maintenance=False) as store:
        store.record_session(session, payload.get("transcript_path"))
        qs = store.hook_candidates(session, service.MAX_HOOK_MISSES)
        claims = {q.id: c for q in qs if (c := store.claim_delivery(q.id))}
        qs = [q for q in qs if q.id in claims]
        if not qs:
            return None
        path = transcript.find(session, payload.get("transcript_path"))
        # a late landing, in any transcript of the conversation: recorded, never sent twice
        landed = service.landed_across(store, qs, [path])
        for q in qs:
            if q.id in landed:
                store.record_delivery(q.id, None, acknowledge=bool(landed[q.id] - {service.REFERENCE_INDEX}),
                                      claim=claims[q.id], verified=True)
        qs = [q for q in qs if q.id not in landed]
        if not qs:
            return None
        text, batch, _ = service.delivery_messages(qs)[0]
        for q in qs:
            if q not in batch:  # the next firing carries it
                store.release_claim(q.id, claims[q.id], q.delivery_error)
        sent = [q for q in batch if store.record_hook_delivery(q.id, claims[q.id])]
        if not sent:
            return None
        if len(sent) != len(batch):
            text = service.delivery_messages(sent)[0][0]
    return herdr.clean(text, limit=10**9)


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
        elif event == "rewake-watch":  # asyncRewake: stdout + the launcher's wake exit
            from . import rewake

            text = rewake.run(payload)
            if not text:
                return 0
            try:
                sys.stdout.write(text + "\n")
                sys.stdout.flush()
            except BaseException:  # noqa: BLE001
                return 0
            return rewake.WAKE_EXIT
        elif event in DELIVERY_EVENTS:
            name = DELIVERY_EVENTS[event]
            context = deliver(payload)
            if not context:
                return 0
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
