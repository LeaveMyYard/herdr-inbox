"""Claude Code hooks: inject the inbox rules into every session, and (0.9.0) hand answers
to a busy session as hook context at its next tool batch or turn end (``deliver``).

Must be fast and must never fail the session: any error means exit 0 with no
output. Deliberately imports nothing heavy (no mcp, no textual).
"""

from __future__ import annotations

import json
import os
import sys

from . import decision_trace  # stdlib only; inert outside tests

_obs = decision_trace.note

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
BUSY_STEP = 0.01  # the wall-clock bounded retry's sleep between attempts (_bounded)


def _bounded(connect, work, timeout: float = PENDING_TIMEOUT):
    """``work(db)`` on a fresh ``connect()`` (opened with sqlite's own busy timeout at 0),
    retried on a busy / locked DB until ``timeout`` seconds of WALL CLOCK have passed, then
    the last sqlite3.Error is raised.

    Not sqlite's busy timeout: that adds up the sleeps it asked for, not the time they took.
    On macOS runners a 0.2 s busy timeout held the rules ~0.9 s on a locked DB (issue #45).
    ``work`` may run more than once; everything it does must be safe to repeat."""
    import sqlite3
    import time

    deadline = time.monotonic() + timeout
    while True:
        try:
            db = connect()
            try:
                return work(db)
            finally:
                db.close()
        except sqlite3.OperationalError as exc:
            code = getattr(exc, "sqlite_errorcode", None)
            busy = code in (sqlite3.SQLITE_BUSY, sqlite3.SQLITE_LOCKED) if code is not None else (
                "locked" in str(exc) or "busy" in str(exc))
            left = deadline - time.monotonic()
            if not busy or left <= 0:
                raise
            time.sleep(min(BUSY_STEP, left))


def _pending(session_id: str) -> list[str]:
    """This session's open / unseen questions, read-only (no schema work, no migration) and
    with a short timeout; [] if the DB is missing, locked or unreadable."""
    import sqlite3

    from .store import db_path

    path = db_path()
    if not path.exists():
        return []
    try:
        rows = _bounded(lambda: sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=0), lambda db: db.execute(
            "SELECT id, status, question, answer FROM questions WHERE session_id = ?"
            " AND (status = 'open' OR (status = 'answered' AND acknowledged_at IS NULL)) ORDER BY id",
            (session_id,),
        ).fetchall())
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

        # never migrates (store.py): on an older schema this raises and run() leaves it
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
    assert column in ("hook_seen_at", "watch_seen_at")

    def stamp_it(db) -> None:  # an upsert of the same stamp: safe to repeat (_bounded)
        with db:
            db.execute(f"INSERT INTO sessions (session_id, {column}) VALUES (?, ?) ON CONFLICT(session_id)"
                       f" DO UPDATE SET {column} = excluded.{column}",
                       (session_id, stamp.isoformat(timespec="seconds")))

    try:  # the common case is a read-only look: the stamp is under a minute old
        row = _bounded(lambda: sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=0), lambda ro: ro.execute(
            f"SELECT {column} FROM sessions WHERE session_id = ?", (session_id,)).fetchone())
        if row and row[0] and row[0] >= cutoff:
            return  # fresh enough: no write, no writable connection
        _bounded(lambda: sqlite3.connect(str(path), timeout=0), stamp_it)
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
    from .store import MAX_HOOK_MISSES  # stdlib-light: not .service

    # the same filter as Store.hook_candidates: rows at the miss cap or whose agent is
    # gone never open the writable store (review #26 L2)
    from .store import family

    def look(db) -> bool:  # reads only: safe to repeat (_bounded)
        # this session and those it continued, while their transcripts still say so
        fam = family(db, session_id, lambda s: (db.execute(
            "SELECT transcript_path FROM sessions WHERE session_id = ?", (s,)).fetchone() or [None])[0])
        return db.execute(
            f"SELECT 1 FROM questions WHERE session_id IN ({','.join('?' * len(fam))}) AND status = 'answered'"
            " AND delivered_at IS NULL AND acknowledged_at IS NULL AND id NOT IN (SELECT id FROM question_ext"
            " WHERE COALESCE(hook_misses, 0) >= ? OR agent_gone_at IS NOT NULL) LIMIT 1",
            (*fam, MAX_HOOK_MISSES)).fetchone() is not None

    try:
        return _bounded(lambda: sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=0), look)
    except sqlite3.Error:
        return False


def deliver(payload: dict, liveness: bool = True) -> str | None:
    """This session's waiting answers as the hook's additionalContext, or None.

    Same text as typing (``[inbox] Q7 answered — …``), one message per firing. Each row is
    claimed (CAS) and recorded as hook-sent BEFORE the text is printed; the delivery worker
    verifies it in the transcript, or puts it back to pending (DESIGN.md, "Hook delivery")."""
    frame = decision_trace.begin("hook", liveness=liveness)
    try:
        session = payload.get("session_id")
        if not _obs(frame, "own_session", bool(session) and not payload.get("agent_id")):
            return None  # a subagent's or teammate's hook: the answer belongs to the session that asked
        if liveness:  # the PostToolBatch/Stop hooks only; the rewake watcher stamps its own column
            _touch(session)
        if not _obs(frame, "has_candidates", _has_candidates(session)):
            return None
        from . import herdr, service, transcript
        from .store import SchemaNotReady, Store

        try:  # never migrates: an older schema waits for the worker or the popup (store.py)
            store = Store(timeout=RECORD_TIMEOUT, maintenance=False)
        except SchemaNotReady:
            _obs(frame, "schema_ready", False)
            return None
        with store:
            store.record_session(session, payload.get("transcript_path"))
            qs = store.hook_candidates(session, service.MAX_HOOK_MISSES)
            decision_trace.track(frame, store, qs)
            claims = {q.id: c for q in qs if (c := store.claim_delivery(q.id))}
            if frame is not None:
                for q in qs:
                    frame.note("claimed", q.id in claims, [q.id])
            qs = [q for q in qs if q.id in claims]
            if not qs:
                return None
            path = transcript.find(session, payload.get("transcript_path"))
            # a late landing, in any transcript of the conversation: recorded, never sent twice
            landed = service.landed_across(store, qs, [path])
            if frame is not None:
                frame.note("transcript_known", path is not None)
                for q in qs:
                    frame.note("landed", landed.get(q.id), [q.id])
            for q in qs:
                if q.id in landed:
                    store.record_delivery(q.id, None, acknowledge=bool(landed[q.id] - {service.REFERENCE_INDEX}),
                                          claim=claims[q.id], verified=True)
            qs = [q for q in qs if q.id not in landed]
            if not qs:
                return None
            text, batch, _ = service.delivery_messages(qs)[0]
            for q in qs:
                _obs(frame, "in_this_message", q in batch, [q.id])
                if q not in batch:  # the next firing carries it
                    store.release_claim(q.id, claims[q.id], q.delivery_error)
            sent = [q for q in batch if _obs(frame, "recorded", store.record_hook_delivery(q.id, claims[q.id]), [q.id])]
            if not sent:
                return None
            if len(sent) != len(batch):
                text = service.delivery_messages(sent)[0][0]
        return herdr.clean(text, limit=10**9)
    finally:
        decision_trace.end(frame)


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
                # first, and cheap: a resumed OLD session id runs itself from now on, not
                # the session its transcript once continued in (store.live_session). Only a
                # resume re-runs an id that may have continued elsewhere: a compaction
                # continuation fires SessionStart:compact (captured, c5240607 10-15 18:22),
                # and clear/fork/startup run a fresh id (review #53 final M1)
                if payload.get("source") == "resume":
                    from .store import record_start

                    record_start(payload.get("session_id"), os.environ.get("HERDR_PANE_ID"))
            except BaseException:  # noqa: BLE001
                pass
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
