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
from datetime import datetime, timezone
import os
from dataclasses import dataclass

from . import herdr, proc, tabs, trace, transcript
from .store import PENDING, REQUEUED, Question, Store, normalize

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
# How long the typed answer may take to show up in the session transcript.
VERIFY_SECONDS = 15.0
NOT_VERIFIED = "not verified: herdr accepted the text but it never reached the conversation"
# Typed but never seen in the transcript: try at most this many times in all, then leave it
# as a ⚠ entry (unseen, for check_answers) instead of typing it again and again -- a verifier
# miss must never turn into the same answer being typed every tick.
MAX_UNVERIFIED_ATTEMPTS = 2
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
    if pane_id and proc.nested_claude():
        pane_id = None  # a claude run by the pane's agent is not the pane's agent
    ident = herdr.pane_identity(pane_id) if pane_id else {}
    session_id = ident.get("session_id") or os.environ.get("CLAUDE_CODE_SESSION_ID") or None
    return Caller(
        pane_id=pane_id,
        tab_id=ident.get("tab_id") or os.environ.get("HERDR_TAB_ID") or None,
        tab_label=_base_label(ident.get("tab_label"), ident.get("tab_id")),  # never our "❓2 " prefix
        session_id=session_id,
        cwd=os.getcwd(),
    )


def _base_label(label: str | None, tab_id: str | None) -> str | None:
    if label is None:
        return None
    with Store() as store:
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


def deliver_batch(store: Store, qs: list[Question], retry: bool = False, force: bool = False) -> None:
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
            for q in items:
                record(q, f"{NO_AGENT} {pane}")
            continue
        ours = []
        for q in items:
            if q.session_id and agent["session_id"] != q.session_id:
                record(q, f"{CHANGED} {pane}")
            else:
                ours.append(q)
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
        if path is not None:
            # already in the conversation? (a late landing, or a sender that typed and then
            # died mid-verify, whose stale claim we just took over): look before EVERY typing
            landed = transcript.landed_exact(path, {q.id: (expected_segments(q), q.answered_at) for q in ours})
            for q in ours:
                if q.id in landed:
                    record(q, None, verified=True)
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
            if path is None:
                # Cannot verify (no transcript known): sent, but never counted as seen, so
                # check_answers still returns it. Not retried (that could double it).
                for q in batch:
                    record(q, None, acknowledge=False)
                continue
            with trace.step("deliver.verify_wait"):
                landed_now = transcript.wait_landed(path, offset, [q.id for q in batch], VERIFY_SECONDS)
            if landed_now:
                for q in batch:
                    record(q, None, acknowledge=ack, verified=True)
            elif busy:  # not counted toward the idle cap (see MAX_BUSY_ATTEMPTS)
                for q in batch:
                    store.record_busy_miss(q.id, claims[q.id], BUSY_WAIT_IDLE)
            else:
                for q in batch:
                    record(q, NOT_VERIFIED)


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
            with Store(path, timeout=5) as own:
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
