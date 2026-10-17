"""Inbox operations: store changes plus their herdr side effects.

Used by the MCP server, the TUI and the CLI alike. herdr failures never
propagate; they are logged in ``herdr.call`` and, for answer delivery,
recorded on the question.
"""

from __future__ import annotations

import logging
import queue
import threading
import time
from datetime import datetime, timedelta, timezone
import os
from dataclasses import dataclass

from . import herdr, proc, tabs, trace, transcript
from .store import MAX_HOOK_MISSES as st_MAX_HOOK_MISSES, PENDING, REQUEUED, Question, Store, normalize

NO_AGENT = "no live agent in pane"
CHANGED = "conversation changed in pane"
# Retries (30 s apart) before giving up on a pane whose conversation is gone;
# the answer stays available through check_answers either way.
MAX_STALE_RETRIES = 10
# herdr agent states in which typing a prompt is safe.
DELIVERABLE = ("idle", "done")
# A prompt or dialog: typed text would be swallowed, so wait however long it takes.
BLOCKED = "blocked"
# Any other state (working): herdr says "working" for as long as Claude Code waits on a
# background workflow (hours), yet typed text is accepted then (queued mid-turn; probe
# 2026-10-14). Wait this long, then type anyway: the transcript check (2-attempt cap) is
# the safety net for a state that swallows input.
BUSY_GRACE = 20.0
# Typed while busy but not seen in the transcript: try this many times while busy, then wait
# for idle. Busy misses never count toward MAX_UNVERIFIED_ATTEMPTS, so an answer always gets
# its tries at idle/done before it is parked as ⚠.
MAX_BUSY_ATTEMPTS = 1  # 0.6.1: one busy try, then the next idle/done (live: the panel ate it)
BUSY_WAIT_IDLE = "busy: typed while the agent was busy but never seen; waiting for it to be idle"
BUSY_MISS = "busy: typed while the agent was busy, not seen yet; trying again"
PANEL = "blocked: panel: the agent's background-task panel has focus"
COMPOSER_DRAFT = "waiting: the agent's input box holds unsent text (send or clear it there)"
COMPOSER_UNKNOWN = "waiting: cannot read the agent's input box"
UNREADABLE_NOTIFY_AFTER = 60.0  # an input box unreadable this long gets ONE notification saying why (#37 L2)
_unreadable_since: dict[str, float] = {}  # pane -> first unreadable read (this process)
_unreadable_notified: set[str] = set()
# How long the typed answer may take to show up in the session transcript.
VERIFY_SECONDS = 15.0
NOT_VERIFIED = "not verified: herdr accepted the text but it never reached the conversation"
# Typed but never seen in the transcript: try at most this many times in all, then leave it
# as a ⚠ entry (unseen, for check_answers) instead of typing it again and again -- a verifier
# miss must never turn into the same answer being typed every tick.
MAX_UNVERIFIED_ATTEMPTS = 2
# 0.9.0 hook delivery (DESIGN.md "Hook delivery to busy Claude sessions")
HOOK_VERIFY_SECONDS = 45.0  # a hook-sent answer must show in the transcript within this
MAX_HOOK_MISSES = st_MAX_HOOK_MISSES  # then the hook leaves that answer to typing
HOOK_WAIT = "busy: delivered at the agent's next tool call or turn end"
HOOK_UNSEEN = "hook: not seen in the conversation; trying again"
HOOK_LIVE = timedelta(minutes=10)  # a working session counts as hook-delivered only if its hook fired this recently
WATCHER_WAIT = "idle: the session's own hook wakes it with the answer"
MAX_HEADER = 12  # AskUserQuestion's chip limit
MAX_OPTIONS = 9  # number keys 1-9
MAX_QUESTIONS = 4  # AskUserQuestion's limit per call

log = logging.getLogger("herdr_inbox")


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
    with trace.step("caller.nested_claude"):
        nested = bool(pane_id) and proc.nested_claude()
    if nested:
        pane_id = None  # a claude run by the pane's agent is not the pane's agent
    with trace.step("caller.pane_identity"):
        ident = herdr.pane_identity(pane_id) if pane_id else {}
    if pane_id and ident.get("exists") is False:
        # HERDR_PANE_ID is the pane this process STARTED in; herdr gives a moved pane a new id
        # (w6:p48 -> w6:p57, live 2026-10-17: Q158-Q166 were filed under the dead id). Find
        # the pane that runs our conversation now.
        moved = _pane_running(os.environ.get("CLAUDE_CODE_SESSION_ID"))
        if moved:
            pane_id = moved
            ident = herdr.pane_identity(pane_id)
    session_id = ident.get("session_id") or os.environ.get("CLAUDE_CODE_SESSION_ID") or None
    if session_id:
        session_id = transcript.follow(session_id)  # a continued conversation files under its live session
    return Caller(
        pane_id=pane_id,
        tab_id=ident.get("tab_id") or os.environ.get("HERDR_TAB_ID") or None,
        tab_label=_base_label(ident.get("tab_label"), ident.get("tab_id")),  # never our "❓2 " prefix
        session_id=session_id,
        cwd=os.getcwd(),
    )


def _pane_running(session_id: str | None) -> str | None:
    """The one pane whose agent runs ``session_id``'s conversation (continuations followed
    on both sides), or None."""
    if not session_id:
        return None
    live = transcript.follow(session_id)
    res, _ = herdr.call("agent", "list")
    hits = [a["pane_id"] for a in (res or {}).get("agents") or []
            if isinstance(a, dict) and a.get("pane_id")
            and transcript.follow((a.get("agent_session") or {}).get("value")) == live]
    return hits[0] if len(hits) == 1 else None


def _base_label(label: str | None, tab_id: str | None) -> str | None:
    if label is None:
        return None
    with trace.step("caller.base_label_store"), Store(timeout=3, maintenance=False) as store:
        return tabs.base_label(label, tabs.marks_for(store, tab_id))


def first_line(text: str, width: int) -> str:
    line = text.strip().splitlines()[0] if text.strip() else ""
    return line if len(line) <= width else line[: width - 1] + "…"


UNAVAILABLE = {
    "status": "unavailable",
    "message": "The operator inbox only serves the top-level agent of a herdr pane, and this "
    "session is not one (not in herdr, or nested inside another agent). Ask in chat, or ask "
    "your caller, instead.",
}


def ask(
    store: Store,
    caller: Caller,
    question: str,
    options: list | None = None,
    context: str | None = None,
    asked_by: str | None = None,
    header: str | None = None,
    multi_select: bool = False,
    title: str | None = None,
) -> dict:
    """File one question (a group of one); the receipt keeps its single-question shape."""
    out = ask_group(store, caller, [{"question": question, "options": options, "header": header,
                                     "multi_select": multi_select}], context, asked_by, title=title)
    return out["questions"][0] if "questions" in out else out


def ask_group(
    store: Store,
    caller: Caller,
    items: list[dict],
    context: str | None = None,
    asked_by: str | None = None,
    title: str | None = None,
) -> dict:
    """File several related questions at once (AskUserQuestion's ``questions``).

    Each item gets its own Q id and is deduped on its own; new ones share a group id and
    the call's ``title`` (shown as the group's name in the popup).
    """
    if not caller.pane_id:
        # Nobody watches the inbox for agents outside herdr: no popup, no notification, no delivery.
        return dict(UNAVAILABLE)
    receipts, new = [], []
    with store.transaction():  # dedupe + insert atomically (parallel calls, subagents)
        group_id = None
        for item in items:
            text = item["question"].strip()
            dup = store.find_duplicate(normalize(text), caller.session_id, caller.pane_id)
            if dup and dup.status == "open":
                store.bump_repeat(dup.id)
                receipts.append({"id": dup.id, "status": "already_open", "message":
                                 f"{dup.ref} is already with the operator. Do not ask again; keep working."})
                continue
            if dup:
                store.acknowledge([dup.id])
                receipts.append({"id": dup.id, "status": "answered", "answer": dup.answer, "message":
                                 f"{dup.ref} was already answered by the operator: {dup.answer}"})
                continue
            q = store.insert(
                text,
                options=item.get("options") or None,
                context=context,
                asked_by=asked_by,
                pane_id=caller.pane_id,
                tab_id=caller.tab_id,
                tab_label=caller.tab_label,
                session_id=caller.session_id,
                cwd=caller.cwd,
                header=(item.get("header") or "").strip()[:MAX_HEADER] or None,
                multi_select=bool(item.get("multi_select")),
                group_id=group_id,
            )
            store.set_server(q.id, herdr.server_id())  # this server's delivery worker owns it
            group_id = q.group
            new.append(q)
            receipts.append({"id": q.id, "status": "open", "message": (
                f"Recorded as {q.ref}. The operator will answer asynchronously; the answer will arrive in "
                f"this conversation as a message starting with `[inbox]` that names {q.ref}. Continue with "
                f"work that does not depend on it. Do not repeat the question in chat or call ask_operator "
                f"for it again. If an answer seems overdue, call check_answers.")})
        if new:
            store.set_group_title(group_id, title)
    if new:
        refs = ", ".join(q.ref for q in new)
        title, body = f"{refs} · {new[0].tab_label or new[0].pane_id}", notification_body(store.group(group_id))
        later(lambda: herdr.notify(title, body))
        with trace.step("ask.markers"):
            update_markers(store, caller.pane_id)
    ids = ", ".join(f"Q{r['id']} ({r['status']})" for r in receipts)
    return {
        "group": group_id,
        "questions": receipts,
        "message": f"Filed {ids}. Answers arrive later as `[inbox]` messages naming the Q ids; keep working "
        "and do not ask these again. If an answer seems overdue, call check_answers.",
    }


def group_name(qs: list[Question]) -> str:
    """The group's name: its title; else (old sessions, single questions) the first
    question's header, else the first line of the first question."""
    if not qs:
        return ""
    first = qs[0]
    return first.group_title or first.header or first_line(first.question, 80)


def notification_body(qs: list[Question]) -> str:
    """The group's title; for an untitled call its first question, with its header if any
    (a header alone, e.g. "DB choice", says too little in a toast)."""
    if not qs:
        return ""
    first = qs[0]
    if first.group_title:
        return first.group_title
    text = first_line(first.question, 200)
    return f"{first.header}: {text}" if first.header else text


def _quote(label: str) -> str:
    """Quote a picked label that contains our separators, so the agent can split picks."""
    if any(ch in label for ch in ',"()·'):
        return '"' + label.replace('"', '\\"') + '"'
    return label


def format_answer(choices: list[str], notes: dict[str, str] | None = None, text: str | None = None) -> str:
    """One line: ``Postgres (note: keep the old schema)``, picks joined by ``, ``."""
    notes = notes or {}
    parts = [f"{_quote(c)} (note: {notes[c]})" if notes.get(c) else _quote(c) for c in choices]
    if text and text.strip():
        parts.append(text.strip())
    return ", ".join(parts)


def make_detail(text: str | None = None, choices: list[str] | None = None,
                notes: dict[str, str] | None = None) -> dict | None:
    """The structured answer (draft or final): picked labels, their notes, free text."""
    choices = list(choices or [])
    notes = {k: v.strip() for k, v in (notes or {}).items() if k in choices and v and v.strip()}
    text = (text or "").strip() or None
    if not choices and not text:
        return None
    return {"choices": choices, "notes": notes, "text": text}


def save_draft(store: Store, qid: int, detail: dict | None) -> bool:
    """Keep the operator's answer without sending it (None clears the draft)."""
    return store.set_draft(qid, detail)


def submit(store: Store, qids: list[int], deliver: bool = True) -> list[Question]:
    """Send the drafted answers among ``qids``; undrafted or closed questions are skipped.

    Delivery is batched: one `[inbox]` message per pane with all of its answers.
    ``deliver=False`` only records the answers (the popup hands delivery to a detached
    ``inbox deliver`` process and closes at once).
    """
    answered = []
    for qid in qids:
        q = store.get(qid)
        if q is None or q.status != "open" or not q.draft_json:
            continue
        d = q.draft_json
        if store.set_answer(qid, format_answer(d.get("choices") or [], d.get("notes"), d.get("text")), d):
            answered.append(store.get(qid))
    if deliver:
        deliver_batch(store, answered)
    for pane in dict.fromkeys(q.pane_id for q in answered):
        update_markers(store, pane)
    return [store.get(q.id) for q in answered]


def answer(
    store: Store,
    qid: int,
    text: str | None = None,
    *,
    choices: list[str] | None = None,
    notes: dict[str, str] | None = None,
    deliver: bool = True,
) -> Question | None:
    """Answer one open question right away (the CLI path): draft + submit. None if not open."""
    detail = make_detail(text, choices, notes)
    if detail is None or not store.set_draft(qid, detail):
        return None
    done = submit(store, [qid], deliver=deliver)
    return done[0] if done else None


REFERENCE_INDEX = 2  # in expected_segments: the long-answer reference (not the answer itself)


def expected_segments(q: Question) -> list[str]:
    """The exact strings a delivery of ``q`` carries (single message, batch segment, or the
    long-answer reference), as herdr types them (``herdr.clean``: tabs, newlines, U+2028
    become spaces): what the transcript must show for ``q`` to count as landed."""
    return [herdr.clean(s, limit=10**9) for s in (delivery_text([q]), answer_line(q), _reference(q))]


def answer_line(q: Question) -> str:
    return f"{q.ref} {first_line(q.question, 80)}: {q.answer}"


def delivery_text(qs: list[Question]) -> str:
    if len(qs) == 1:
        q = qs[0]
        return f"[inbox] {q.ref} answered — {first_line(q.question, 80)}: {q.answer}"
    return "[inbox] answered — " + " · ".join(answer_line(q) for q in qs)


def _too_long(q: Question) -> bool:
    return len(delivery_text([q])) > herdr.MAX_ARG


def _reference(q: Question) -> str:
    return (f"[inbox] {q.ref} answered — {first_line(q.question, 80)}: (long answer, {len(q.answer)} chars; "
            f"call check_answers to read it)")


def delivery_messages(qs: list[Question]) -> list[tuple[str, list[Question], bool]]:
    """Split one pane's answers into messages that fit herdr's argument cap, never truncating.

    Returns (text, questions, acknowledged): an answer too long for any message is sent as a
    reference and stays unacknowledged, so check_answers still returns it.
    """
    out: list[tuple[str, list[Question], bool]] = []
    batch: list[Question] = []
    for q in qs:
        if _too_long(q):
            out.append((_reference(q), [q], False))
            continue
        if batch and len(delivery_text(batch + [q])) > herdr.MAX_ARG:
            out.append((delivery_text(batch), batch, True))
            batch = []
        batch.append(q)
    if batch:
        out.append((delivery_text(batch), batch, True))
    return out


def deliver(store: Store, q: Question, retry: bool = False, force: bool = False) -> None:
    deliver_batch(store, [q], retry, force)


def spawn_cli(args: list[str]) -> None:
    """Start ``inbox <args>`` detached (own session, outlives the caller), without the
    caller's venv selection, logging to worker.log; an OSError is logged, not raised."""
    import subprocess
    import sys

    from . import worker

    log_path = worker.db_path().parent / "worker.log"
    try:
        with open(log_path, "a") as err:
            subprocess.Popen([sys.executable, "-m", "herdr_inbox.cli", *args], stdin=subprocess.DEVNULL,
                             stdout=subprocess.DEVNULL, stderr=err, start_new_session=True, close_fds=True,
                             env=worker._spawn_env())
    except OSError:
        log.warning("could not start `inbox %s`", " ".join(args[:1]), exc_info=True)


def _ensure_worker(server: str | None = None) -> None:
    try:
        from . import worker

        worker.ensure(server)
    except Exception as exc:  # noqa: BLE001 - never fail a send over it
        log.warning("delivery worker: %s", exc)


def _busy_for(store: Store, qid: int) -> float:
    """Seconds this answer has been pending (0 if it is not)."""
    since = store.get_pending_since(qid)
    if not since:
        return 0.0
    return (datetime.now(timezone.utc) - datetime.fromisoformat(since)).total_seconds()


def deliver_batch(store: Store, qs: list[Question], retry: bool = False, force: bool = False,
                  verify_later=None) -> None:
    """``force``: the operator's "Send now" -- skip the busy grace and the focused-pane wait
    (never a blocked agent: a dialog would swallow the text)."""
    if not retry:
        _ensure_worker()  # whatever this send leaves pending, the worker retries
    mine = herdr.server_id()
    foreign = [q for q in qs if store.server_of(q.id) not in (None, mine)]
    for server in {store.server_of(q.id) for q in foreign}:
        _ensure_worker(server)  # asked on another herdr server: its own worker delivers it
    qs = [q for q in qs if q not in foreign]
    """Type answers into the asking conversations: one message per pane, only into the
    conversation that asked.

    A pane can hold a different conversation by now (/clear, restart, another agent);
    those answers stay for check_answers and the compact/resume hook.
    """
    by_pane: dict[str | None, list[Question]] = {}
    for q in qs:
        by_pane.setdefault(q.pane_id, []).append(q)
    for pane, items in by_pane.items():
        # Claim every row before writing any outcome for it (sent, no agent, conversation
        # changed): another process mid-send is never overwritten, and never doubled.
        claims = {q.id: c for q in items if (c := store.claim_delivery(q.id))}
        items = [q for q in items if q.id in claims]
        if not items:
            continue

        def record(q: Question, error: str | None, acknowledge: bool = True, verified: bool = False) -> None:
            store.record_delivery(q.id, error, acknowledge=acknowledge, claim=claims[q.id], verified=verified)

        if not pane:
            for q in items:
                record(q, "no pane: answer available through check_answers")
            continue
        agent = herdr.live_agent(pane)
        if agent is None:
            for q in _relocate(store, items, pane, claims):  # its pane may only have moved
                record(q, f"{NO_AGENT} {pane}")
            continue
        ours, changed = [], []
        for q in items:
            if q.session_id and agent["session_id"] != q.session_id \
                    and transcript.follow(q.session_id, store.session_transcript(q.session_id)) != agent["session_id"]:
                changed.append(q)
            else:
                ours.append(q)
        for q in _relocate(store, changed, pane, claims) if changed else []:  # moved to another pane?
            record(q, f"{CHANGED} {pane}")
        if not ours:
            continue
        status = agent["status"]
        if status == BLOCKED:
            # At a prompt or dialog, typed text is swallowed: wait (the retry loop delivers
            # once it is answered), however long that takes.
            for q in ours:
                store.set_pending(q.id, claims[q.id], "blocked: the agent is at a prompt or dialog")
            continue
        busy = status not in DELIVERABLE
        if status in DELIVERABLE and not force and _watched(store, agent["session_id"]):
            # idle, and its asyncRewake watcher is alive: it wakes the session with the answer
            # (no keystrokes); answers its hooks missed twice fall through to typing
            watched = [q for q in ours if store.hook_misses(q.id) < MAX_HOOK_MISSES]
            for q in watched:
                store.set_pending(q.id, claims[q.id], WATCHER_WAIT)
            ours = [q for q in ours if q not in watched]
            if not ours:
                continue
        if status == "working" and not force and store.hook_live(agent["session_id"], HOOK_LIVE):
            # its own hook carries the answer at the next tool batch or turn end: never type
            # into a working Claude that has it (answers its hook missed twice fall through)
            hooked = [q for q in ours if store.hook_misses(q.id) < MAX_HOOK_MISSES]
            for q in hooked:
                store.set_pending(q.id, claims[q.id], HOOK_WAIT)
            ours = [q for q in ours if q not in hooked]
            if not ours:
                continue
        if busy and not force:  # requeued after an old server's lost delivery: at rest only
            requeued = [q for q in ours if (q.delivery_error or "") == REQUEUED]
            for q in requeued:
                store.set_pending(q.id, claims[q.id], REQUEUED[len(PENDING):])
            ours = [q for q in ours if q not in requeued]
            if not ours:
                continue
        if busy:
            # Give it BUSY_GRACE to come to rest, then type anyway (verified below); after
            # MAX_BUSY_ATTEMPTS misses while busy, wait for idle. "Send now" skips the grace.
            waiting = []
            for q in ours:
                if not force and store.busy_attempts(q.id) >= MAX_BUSY_ATTEMPTS:
                    store.set_pending(q.id, claims[q.id], BUSY_WAIT_IDLE)
                    waiting.append(q)
                elif not force and _busy_for(store, q.id) < BUSY_GRACE:
                    store.set_pending(q.id, claims[q.id], f"busy: the agent is {status}")
                    waiting.append(q)
            ours = [q for q in ours if q not in waiting]
            if not ours:
                continue
        if retry and agent["focused"] and not force:
            # The operator may be typing there: the prompt would merge with their input.
            for q in ours:
                store.set_pending(q.id, claims[q.id], "waiting: the operator is in that pane")
            continue
        if not force and herdr.panel_focused(pane):
            # Claude Code's background-agents panel (or a subagent view) has focus: typed
            # text never reaches the conversation (probe 2026-10-15). Wait, even for Send now.
            for q in ours:
                store.set_pending(q.id, claims[q.id], PANEL)
            continue
        path = transcript.find(agent["session_id"], store.session_transcript(agent["session_id"]))
        # already in the conversation? (a late landing, or a sender that typed and then
        # died mid-verify, whose stale claim we just took over): look before EVERY typing,
        # in every transcript of the conversation (a continuation moved it, #41)
        # every transcript of the conversation: its chain, resumed predecessors, what the pane
        # runs (operator 2026-10-17: never deliver twice what already arrived anywhere)
        pane_sessions = [s for s in (agent.get("session_id"), agent.get("reported_session_id")) if s]
        related = {str(p): p for q in ours for p in _relative_paths(store, q.session_id, pane_sessions)}
        landed = landed_across(store, ours, [path] + [p for p, _ in _watch_paths(store, agent)] + list(related.values()))
        # (no "seen another way" evidence here: live, `inbox answer` from the conversation
        # marks the row seen itself; that transcript heuristic only reconciles old rows)
        for q in ours:
            if q.id in landed:
                # only the long-answer reference landed: the agent has a pointer to
                # check_answers, not the answer -- verified, but it stays unseen
                record(q, None, acknowledge=bool(landed[q.id] - {REFERENCE_INDEX}), verified=True)
        ours = [q for q in ours if q.id not in landed]
        if not ours:
            continue
        for text, batch, ack in delivery_messages(ours):
            if retry and not force:
                # the operator may have stepped into the pane since the check above
                now_agent = herdr.live_agent(pane)
                if now_agent is None or now_agent["focused"] or now_agent["session_id"] != agent["session_id"]:
                    for q in batch:
                        store.set_pending(q.id, claims[q.id], "waiting: the operator is in that pane")
                    continue
            if agent.get("agent") == "claude":
                # never type onto unsent text in Claude Code's input box: it would be appended
                # to the draft and submitted with it (live 2026-10-16, Q124/Q125). Not even
                # for Send now; an input box we cannot read counts as not empty.
                state = herdr.composer_state(pane)
                _note_unreadable(pane, state, batch)
                if state != "empty":
                    reason = COMPOSER_DRAFT if state == "draft" else COMPOSER_UNKNOWN
                    for q in batch:
                        store.set_pending(q.id, claims[q.id], reason)
                    if force:
                        refs = ", ".join(q.ref for q in batch)
                        later(lambda refs=refs, reason=reason: herdr.notify(
                            f"{refs} not sent", reason[len("waiting: "):]))
                    continue
            # the transcripts the typed text can land in: the followed conversation's AND the
            # session herdr reports (after herdr resumed an OLD id, the first prompt lands in
            # the OLD file and only then ends its continuation; live 2026-10-17, Q129-Q152)
            watch = _watch_paths(store, agent)
            offset = transcript.size(path)
            with trace.step("deliver.prompt", chars=len(text)):
                err = herdr.prompt_agent(pane, text)
            if err:
                for q in batch:
                    record(q, err)
                continue
            if path is None:  # a brand-new session's transcript may appear with this very prompt
                path = transcript.find(agent["session_id"], store.session_transcript(agent["session_id"]))
                offset = 0
            if path is not None and str(path) not in {str(p) for p, _ in watch}:
                watch.append((path, offset))
            if path is None:
                watch = _watch_paths(store, agent, fresh=True)
                path = watch[0][0] if watch else None
            if path is None:
                # Cannot verify (no transcript known): sent, but never counted as seen, so
                # check_answers still returns it. Not retried (that could double it).
                for q in batch:
                    record(q, None, acknowledge=False)
                continue
            if verify_later is not None:
                # typed now (the popup still holds the keyboard); the up-to-15 s transcript
                # check runs in a detached `inbox verify`, which records under our claims
                verify_later({"path": str(path), "offset": offset, "ack": ack, "busy": busy,
                              "watch": [[str(p), off] for p, off in watch],
                              "text": herdr.clean(text, limit=10**9),
                              "rows": [{"id": q.id, "claim": claims[q.id]} for q in batch]})
                continue
            with trace.step("deliver.verify_wait"):
                landed_now = transcript.wait_landed_any(watch, [q.id for q in batch],
                                                        VERIFY_SECONDS, text=herdr.clean(text, limit=10**9))
            if landed_now:
                for q in batch:
                    record(q, None, acknowledge=ack, verified=True)
            elif busy:  # not counted toward the idle cap (see MAX_BUSY_ATTEMPTS)
                for q in batch:
                    store.record_busy_miss(q.id, claims[q.id], BUSY_WAIT_IDLE)
            else:
                for q in batch:
                    record(q, NOT_VERIFIED)


def _note_unreadable(pane: str, state: str, batch: list[Question]) -> None:
    """Once per pane (per process), when its Claude input box stays unreadable for
    UNREADABLE_NOTIFY_AFTER: a distinct notification, so a stall from a Claude Code UI change
    says why instead of only the generic "not delivered" after 5 minutes."""
    if state != "unknown":
        _unreadable_since.pop(pane, None)
        _unreadable_notified.discard(pane)
        return
    first = _unreadable_since.setdefault(pane, time.monotonic())
    if pane in _unreadable_notified or time.monotonic() - first < UNREADABLE_NOTIFY_AFTER:
        return
    _unreadable_notified.add(pane)
    where = batch[0].tab_label or pane
    refs = ", ".join(q.ref for q in batch)
    later(lambda: herdr.notify(f"{where}: cannot read the agent's input box",
                               f"{refs} wait; no answer is typed until it can be read (Claude Code UI changed? "
                               "check herdr-inbox)"))


def _watched(store: Store, session_id: str | None) -> bool:
    """An idle session is left to its asyncRewake watcher only while the watcher is provably
    alive: it holds the session lock AND it stamped its own ``watch_seen_at`` recently. The
    WORKING gate reads ``hook_seen_at`` (PostToolBatch/Stop firings) instead (#30 M1)."""
    if not store.watch_live(session_id, HOOK_LIVE):
        return False
    from . import rewake

    try:
        return rewake.watcher_alive(session_id)
    except OSError:
        return False


MOVED = "moved: the asking agent is now in pane"
REVIVE_EVERY = 120.0  # per gone answer: look for its conversation in another pane at most this often
_revive_checked: dict[int, float] = {}


def _tab_name(tab_id: str | None) -> str | None:
    """The tab's label without the inbox's own ❓ prefix (for a moved question's tab_label)."""
    if not tab_id:
        return None
    label = herdr.tab_label(tab_id)
    return tabs.base_label(label, {tabs.DEFAULT_PREFIX}) if label else None


def session_pane(store: Store, session_id: str | None, agents: list | None = None) -> tuple[str, str | None] | None:
    """The one pane whose agent runs ``session_id``'s conversation now (continuations
    followed on both sides), or None when there is none or more than one (never a guess)."""
    if not session_id:
        return None
    if agents is None:
        res, _ = herdr.call("agent", "list")
        agents = (res or {}).get("agents") or []
    live = transcript.follow(session_id, store.session_transcript(session_id))
    hits = []
    for a in agents:
        reported = (a.get("agent_session") or {}).get("value") if isinstance(a, dict) else None
        if reported and a.get("pane_id") and transcript.follow(reported) == live:
            hits.append((a["pane_id"], a.get("tab_id")))
    return hits[0] if len(hits) == 1 else None


def _relocate(store: Store, items: list[Question], pane: str, claims: dict) -> list[Question]:
    """Rows whose conversation runs in ANOTHER pane now (the pane moved, e.g. w6:p48 ->
    w6:p57, live 2026-10-16 Q155): moved there, pending for the next pass. Returns the rest."""
    rest, agents = [], None
    for q in items:
        if agents is None:
            res, _ = herdr.call("agent", "list")
            agents = (res or {}).get("agents") or []
        loc = session_pane(store, q.session_id, agents)
        if loc and loc[0] != pane and store.move_to_pane(q.id, loc[0], loc[1], f"{MOVED} {loc[0]}", claims[q.id],
                                                         tab_label=_tab_name(loc[1])):
            continue
        rest.append(q)
    return rest


RECONCILE_EVERY = 300.0  # per failed answer: look for it in the transcripts at most this often
RECONCILE_WINDOW = timedelta(days=7)
GONE_ARCHIVE_AFTER = timedelta(hours=24)  # operator Q167: gone answers leave the popup after a day
RELATIVE_MAX_AGE = 14 * 86400  # transcripts older than this are never scanned for relatives
_reconciled_at: dict[int, float] = {}


def related_sessions(store: Store, session: str | None) -> set[str]:
    """``session``, its continuation chain, and every known session whose transcript EVER
    continued into one of those (an old session herdr resumed afterwards, live 2026-10-17):
    one conversation, as far as who has its answers goes. Never used for ownership."""
    members = set(transcript.chain(session, store.session_transcript(session))) if session else set()
    known = store.db.execute("SELECT session_id, transcript_path FROM sessions WHERE transcript_path IS NOT NULL"
                             ).fetchall()
    fresh = []
    for sid, tp in known:
        p = transcript.find(sid, tp)
        try:
            if p is not None and time.time() - p.stat().st_mtime <= RELATIVE_MAX_AGE:
                fresh.append((sid, p))
        except OSError:
            continue
    grew = bool(members)
    while grew:
        grew = False
        for sid, p in fresh:
            if sid not in members and transcript.continued_targets(p) & members:
                members.add(sid)
                grew = True
    return members


def _relative_paths(store: Store, session: str | None, pane_sessions: list[str]) -> list:
    """Every transcript an answer for ``session`` can have landed in: its related sessions and
    the conversations the asking pane runs now (typed text lands where the pane is). Landing
    still needs our exact text."""
    members = related_sessions(store, session)
    for s in pane_sessions:
        # what the pane runs counts only if it is this conversation (a reused pane's other
        # conversation never is: review #44)
        if s in members or transcript.follow(s) in members:
            members.update(transcript.chain(s))
    paths, seen = [], set()
    for m in members:
        p = transcript.find(m, store.session_transcript(m))
        if p is not None and str(p) not in seen:
            seen.add(str(p))
            paths.append(p)
    return paths


def answered_by_own_conversation(store: Store, q: Question) -> bool:
    """This process runs inside the conversation that asked ``q`` (its continuation, or an
    older session of it resumed), e.g. an agent answering its own question with the CLI."""
    # only a Claude Code session's own process (its Bash tool, hooks) has CLAUDE_CODE_SESSION_ID;
    # the operator's shell, even in a herdr pane, does not
    me = os.environ.get("CLAUDE_CODE_SESSION_ID")
    if not q.session_id or not me:
        return False
    return me in related_sessions(store, q.session_id) or transcript.follow(me) == transcript.follow(q.session_id)


def reconcile_failed(store: Store) -> None:
    """The worker (first tick = on start, then backed off per answer): answers shown as failed
    (not verified, agent gone, conversation changed, no agent) that DID land in a transcript
    of their conversation are recorded delivered and verified; answers already seen lose their
    stale error. Live 2026-10-17: 6 'not verified' answers had landed in a resumed old session."""
    store.clear_seen_errors()
    t = time.monotonic()
    rows = [q for q in store.failed_pile(RECONCILE_WINDOW)
            if store.server_of(q.id) in (None, herdr.server_id())
            and t - _reconciled_at.get(q.id, -RECONCILE_EVERY) >= RECONCILE_EVERY]
    if not rows:
        return
    res, _ = herdr.call("agent", "list")
    by_pane: dict[str, list[str]] = {}
    for a in (res or {}).get("agents") or []:
        s = (a.get("agent_session") or {}).get("value") if isinstance(a, dict) else None
        if s and a.get("pane_id"):
            by_pane.setdefault(a["pane_id"], []).append(s)
    for q in rows:
        _reconciled_at[q.id] = t
        paths = _relative_paths(store, q.session_id, by_pane.get(q.pane_id or "", []))
        found: set[int] = set()
        for p in paths:
            found |= transcript.landed_kinds(p, {q.id: (expected_segments(q), q.answered_at)}).get(q.id, set())
        if found:
            store.mark_landed(q.id, acknowledge=bool(found - {REFERENCE_INDEX}))
        elif any(transcript.seen_answer(p, q.id, q.question, q.answer or "", q.answered_at) for p in paths):
            store.mark_seen_elsewhere(q.id)


def archive_gone(store: Store) -> None:
    store.archive_gone(GONE_ARCHIVE_AFTER)


def _watch_paths(store: Store, agent: dict, fresh: bool = False) -> list:
    """(transcript, current size) for the pane's followed session and herdr's reported one.
    ``fresh``: after typing, a file that did not exist before starts at offset 0."""
    out, seen = [], set()
    for s in (agent.get("session_id"), agent.get("reported_session_id")):
        p = transcript.find(s, store.session_transcript(s)) if s else None
        if p is not None and str(p) not in seen:
            seen.add(str(p))
            out.append((p, 0 if fresh else transcript.size(p)))
    return out


def landed_across(store: Store, qs: list[Question], extra: list | None = None) -> dict[int, set[int]]:
    """landed_kinds over every transcript a question's conversation lived in: the session it
    was filed under and each session it continued in (review #41: a delivery that landed in
    the OLD transcript just before a continuation is found, never sent again), plus ``extra``
    paths (e.g. the hook payload's own transcript)."""
    found: dict[int, set[int]] = {}
    by_session: dict[str | None, list[Question]] = {}
    for q in qs:
        by_session.setdefault(q.session_id, []).append(q)
    paths: dict[str, list[Question]] = {}
    for session, items in by_session.items():
        for s in transcript.chain(session, store.session_transcript(session)) if session else []:
            p = transcript.find(s, store.session_transcript(s))
            if p is not None:
                paths.setdefault(str(p), []).extend(items)
    for p in extra or []:
        if p is not None:
            paths.setdefault(str(p), []).extend(qs)
    from pathlib import Path

    for p, items in paths.items():
        uniq = {q.id: q for q in items}
        for qid, kinds in transcript.landed_kinds(Path(p), {q.id: (expected_segments(q), q.answered_at)
                                                            for q in uniq.values()}).items():
            found.setdefault(qid, set()).update(kinds)
    return found


def revive_moved(store: Store) -> None:
    """The worker: answers marked 'agent gone' in the last day whose conversation turns up in
    another pane of this herdr server are moved there and delivered after all."""
    t = time.monotonic()
    rows = [q for q in store.gone_recent(timedelta(days=1)) if store.server_of(q.id) in (None, herdr.server_id())
            and t - _revive_checked.get(q.id, -REVIVE_EVERY) >= REVIVE_EVERY]  # backed off per answer
    if not rows:
        return
    for q in rows:
        _revive_checked[q.id] = t
    res, _ = herdr.call("agent", "list")
    agents = (res or {}).get("agents") or []
    for q in rows:
        loc = session_pane(store, q.session_id, agents)
        if loc and loc[0] != q.pane_id:
            store.move_to_pane(q.id, loc[0], loc[1], f"{MOVED} {loc[0]}", tab_label=_tab_name(loc[1]))


def adopt_continuations(store: Store) -> None:
    """The worker, every tick: sessions with questions or alerts still in play whose
    transcript says the conversation continued in another session: the continuation is
    recorded (sessions.continued_in). Rows keep their filed session (it is in qkey)."""
    rows = store.db.execute(
        "SELECT DISTINCT session_id FROM questions WHERE session_id IS NOT NULL"
        " AND (status = 'open' OR (status = 'answered' AND acknowledged_at IS NULL))"
        " UNION SELECT DISTINCT session_id FROM alerts WHERE session_id IS NOT NULL AND status = 'open'").fetchall()
    for (session,) in rows:  # every tick, idempotent: rows filed under OLD later are covered too
        found = transcript.chain(session, store.session_transcript(session))
        for old, new in zip(found, found[1:]):
            store.record_continuation(old, new)


def verify_hook_deliveries(store: Store) -> None:
    """The worker, every tick: hook-sent answers found in their transcript become verified
    (and seen); not found within HOOK_VERIFY_SECONDS, back to pending with a miss counted."""
    for q, sent_at in store.hook_unverified(herdr.server_id()):
        path = transcript.find(q.session_id, store.session_transcript(q.session_id))
        landed = landed_across(store, [q])  # the whole continuation chain (#41)
        if q.id in landed:
            store.hook_verified(q.id, acknowledge=bool(landed[q.id] - {REFERENCE_INDEX}))
            continue
        try:
            age = (datetime.now(timezone.utc) - datetime.fromisoformat(sent_at)).total_seconds()
        except ValueError:
            age = HOOK_VERIFY_SECONDS
        if age >= HOOK_VERIFY_SECONDS:
            # no transcript to look in: like typing without one, sent but never counted seen
            store.hook_unseen(q.id, sent_at, HOOK_UNSEEN if path is not None else None)


def retry_deliveries(store: Store, pane_id: str) -> None:
    """Retry failed deliveries for one pane (e.g. the agent sat at a permission dialog)."""
    caps = {NO_AGENT: MAX_STALE_RETRIES, CHANGED: MAX_STALE_RETRIES, NOT_VERIFIED: MAX_UNVERIFIED_ATTEMPTS}
    deliver_batch(store, store.undelivered(pane_id, caps), retry=True)


def close(store: Store, qid: int, status: str, note: str | None = None) -> bool:
    q = store.get(qid)
    if not q or not store.set_status(qid, status, note):
        return False
    update_markers(store, q.pane_id)
    return True


# -- side effects off the caller's thread ---------------------------------------------------
# Tab labels (a lock that can wait 10 s), herdr notifications and the like are cosmetic: the
# MCP tools and the popup must not wait for them. A process that cares (the MCP server, the
# popup) calls enable_background(); the CLI and the tests run them inline.
_bg_queue: "queue.Queue | None" = None


def enable_background() -> None:
    global _bg_queue
    if _bg_queue is not None:
        return
    q: queue.Queue = queue.Queue()

    def run() -> None:
        while True:
            fn = q.get()
            try:
                with trace.step("background.job"):
                    fn()
            except BaseException:  # noqa: BLE001 - one bad job never stops the queue
                log.warning("background job failed", exc_info=True)
            finally:
                q.task_done()

    threading.Thread(target=run, name="inbox-background", daemon=True).start()
    _bg_queue = q


def later(fn) -> None:
    """Run ``fn`` in the background thread if enabled, else now."""
    if _bg_queue is None:
        fn()
    else:
        _bg_queue.put(fn)


def drain(timeout: float) -> bool:
    """Wait up to ``timeout`` for queued background work; True if it all ran."""
    if _bg_queue is None:
        return True
    deadline = time.monotonic() + timeout
    while _bg_queue.unfinished_tasks and time.monotonic() < deadline:
        time.sleep(0.02)
    return not _bg_queue.unfinished_tasks


def update_markers(store: Store, pane_id: str | None) -> None:
    """Refresh the pane's ``$inbox_q`` token and the ``❓N`` prefix of the tab it is in now.

    The token is per pane and idempotent: no lock. The tab label update reads the count,
    re-reads the label and renames under ``tabs.marker_lock`` (a file lock, not the
    database lock), so concurrent processes cannot leave a stale count and a slow herdr
    never blocks database writers.
    """
    if not pane_id:
        return
    if _bg_queue is not None:  # off this thread: its own connection, its own wait for the lock
        path = store.path

        def job() -> None:
            with Store(path, timeout=5, maintenance=False) as own:
                update_markers_now(own, pane_id)

        later(job)
        return
    update_markers_now(store, pane_id)


def update_markers_now(store: Store, pane_id: str) -> None:
    try:
        _update_markers(store, pane_id)
    except Exception:  # noqa: BLE001 - markers are cosmetic: never fail the ask/answer that changed them
        log.warning("could not refresh the inbox markers for %s", pane_id, exc_info=True)


def _update_markers(store: Store, pane_id: str) -> None:
    herdr.set_open_count(pane_id, store.open_counts().get(pane_id, 0))
    mark = tabs.prefix()
    if not mark:  # disabled: stale prefixes go at the next sync
        return
    pane_tabs = herdr.pane_tabs()
    tab_id = (pane_tabs or {}).get(pane_id)
    if not tab_id:
        return
    with tabs.marker_lock(store) as locked:
        if locked:
            tabs.apply(store, tab_id, tabs.tab_counts(store.open_counts(), pane_tabs).get(tab_id, 0), mark)


def sync_tokens(store: Store, remove_prefixes: bool = False) -> None:
    """Reconcile every marker with the database (herdr drops tokens on restart; a crash
    can leave a stale tab prefix): ``inbox_q`` on every pane, the prefix on every tab.
    ``remove_prefixes`` strips every prefix (before uninstalling). One short locked
    section per tab."""
    mark = "" if remove_prefixes else tabs.prefix()
    pane_tabs = herdr.pane_tabs()
    counts = store.open_counts()
    for pane in (list(pane_tabs) if pane_tabs is not None else store.known_panes()):
        herdr.set_open_count(pane, counts.get(pane, 0))
    if pane_tabs is None:
        return
    for tab in herdr.all_tabs() or []:
        if tab.get("label") is None:
            continue
        tab_id = tab["tab_id"]
        with tabs.marker_lock(store) as locked:
            if not locked:
                continue
            count = tabs.tab_counts(store.open_counts(), pane_tabs).get(tab_id, 0)
            want = tabs.desired_label(tabs.base_label(tab["label"], tabs.marks_for(store, tab_id)), count, mark)
            if want != tab["label"] or tabs.marks_for(store, tab_id) - {tabs.DEFAULT_PREFIX}:
                tabs.apply(store, tab_id, count, mark)  # re-reads before renaming


def verify_job(store: Store, job: dict) -> None:
    """``inbox verify``: wait for a typed message in the transcript and record the outcome
    under the claims the typing process took (see deliver_batch's ``verify_later``)."""
    from pathlib import Path

    ids = [r["id"] for r in job["rows"]]
    watch = [(Path(p), int(off)) for p, off in job.get("watch") or []] or [(Path(job["path"]), int(job["offset"]))]
    landed = transcript.wait_landed_any(watch, ids, VERIFY_SECONDS, text=job.get("text"))
    for r in job["rows"]:
        if landed:
            store.record_delivery(r["id"], None, acknowledge=bool(job.get("ack", True)), claim=r["claim"], verified=True)
        elif job.get("busy"):
            store.record_busy_miss(r["id"], r["claim"], BUSY_WAIT_IDLE)
        else:
            store.record_delivery(r["id"], NOT_VERIFIED, claim=r["claim"])
