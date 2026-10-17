"""Keystroke-free delivery to an IDLE Claude session (0.10.0): the asyncRewake watcher.

Claude Code runs a command hook marked ``asyncRewake: true`` in the background; when it
exits 2, its stderr (or, if empty, its stdout) wakes the session as a system reminder and
starts a turn (probed on 2.1.301: an idle session answered 2 s after the exit). So every
``Stop`` arms one watcher for its session (``run``):

- it waits for an answer for this session that is neither delivered nor seen, polling the
  database read-only (``WATCH_POLL``);
- only while herdr reports the pane's agent ``idle``/``done`` with this very session (a
  working session gets it from the PostToolBatch/Stop hooks; a blocked one must not be
  disturbed; a pane holding another conversation ends the watch, checked every
  PANE_CHECK_EVERY even with nothing pending; a pane herdr names no session for is never
  woken, and ends the watch after UNKNOWN_SESSION_END);
- then it claims the rows (CAS), checks the transcript for a late landing, records them
  hook-sent (verified later by the worker, as for the PostToolBatch hook) and returns the
  ``[inbox] …`` text, which the launcher prints with exit 2.

One watcher per session: each new one writes its token to ``<session>.owner``; an older one
that sees another token exits. It also exits when its Claude process is gone, after
``WATCH_MAX_SECONDS``, or when the pane changes conversation. While alive it holds
``<session>.lock`` (flock): the delivery worker does not type into an idle session whose
watcher is alive (``watcher_alive``).
"""

from __future__ import annotations

import fcntl
import os
import time
import uuid
from pathlib import Path

from . import transcript  # stdlib only

WATCH_POLL = 2.0  # database look (read-only); the wake follows within one poll
PANE_CHECK_EVERY = 45.0  # herdr look even with nothing pending: a /clear or /resume ends the watch (#30 M1)
UNKNOWN_SESSION_END = 180.0  # herdr names no session for the pane this long: the watch ends (#30 M1b)
WATCH_MAX_SECONDS = 12 * 3600.0 - 300  # under the hooks.json timeout (43200 s), which Claude Code enforces
LOCK_WAIT = 5.0  # for the superseded watcher to let go
WAKE_EXIT = 3  # run.py's private "wake" code; the hooks.json wrapper maps it (and only it) to 2


def _dir() -> Path:
    from .store import db_path

    d = db_path().parent / "rewake"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _safe(session_id: str) -> str:
    """The lock / owner file name of a session: only ever its canonical id (run and
    watcher_alive never get this far with anything else)."""
    sid = transcript.session_id_or_none(session_id)
    if sid is None:
        raise ValueError(f"not a session id: {session_id!r}")
    return sid


def watcher_alive(session_id: str | None) -> bool:
    """Is a watcher for ``session_id`` alive (holding its lock)?"""
    if transcript.session_id_or_none(session_id) is None:
        return False
    path = _dir() / f"{_safe(session_id)}.lock"
    if not path.exists():
        return False
    try:
        with open(path, "a") as fh:
            try:
                fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                return True
            fcntl.flock(fh, fcntl.LOCK_UN)
            return False
    except OSError:
        return False


def _owner_path(session_id: str) -> Path:
    return _dir() / f"{_safe(session_id)}.owner"


def _claim_ownership(session_id: str) -> str:
    token = uuid.uuid4().hex
    path = _owner_path(session_id)
    tmp = path.with_suffix(f".{token}.tmp")
    tmp.write_text(token)
    os.replace(tmp, path)
    return token


def _still_owner(session_id: str, token: str) -> bool:
    try:
        return _owner_path(session_id).read_text() == token
    except OSError:
        return False


def _ready(pane: str, session_id: str) -> str:
    """'go' (idle/done, this very session), 'wait' (working, blocked, unknown status),
    'unknown' (herdr names no session for the pane: never a wake, #30 M1b), or 'end' (agent
    gone, or the pane holds another conversation now)."""
    from . import herdr

    agent = herdr.live_agent(pane)
    if agent is None:
        return "end"
    if not agent.get("session_id"):
        return "unknown"
    if agent["session_id"] != session_id:
        return "end"
    return "go" if agent.get("status") in ("idle", "done") else "wait"


def run(payload: dict, clock=time.monotonic, sleep=time.sleep) -> str | None:
    """Watch for this session's answers; the wake text, or None (nothing to wake for)."""
    from . import hook, proc

    session = payload.get("session_id")
    pane = os.environ.get("HERDR_PANE_ID")
    if transcript.session_id_or_none(session) is None or not pane or payload.get("agent_id"):
        return None  # the one session-id validator: the id names the lock and owner files
    owner = proc.owning_claude()
    token = _claim_ownership(session)
    lock = open(_dir() / f"{_safe(session)}.lock", "a")
    try:
        deadline = clock() + LOCK_WAIT
        while True:  # the superseded watcher sees the new owner within WATCH_POLL and lets go
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except OSError:
                if clock() >= deadline or not _still_owner(session, token):
                    return None
                sleep(0.1)
        end = clock() + WATCH_MAX_SECONDS
        last_check, unknown_since = clock(), None
        while clock() < end:
            if not _still_owner(session, token) or not proc.alive(owner):
                return None
            pending = hook._has_candidates(session)
            if pending or clock() - last_check >= PANE_CHECK_EVERY:
                last_check = clock()
                state = _ready(pane, session)
                if state == "end":
                    return None  # also with nothing pending: a /clear or /resume left this watch behind
                if state == "unknown":
                    unknown_since = unknown_since if unknown_since is not None else clock()
                    if clock() - unknown_since >= UNKNOWN_SESSION_END:
                        return None
                else:
                    unknown_since = None
                if state == "go" and pending:
                    text = hook.deliver(payload, liveness=False)
                    if text:
                        return text
            hook._touch(session, "watch_seen_at")  # the IDLE gate's own stamp (a write a minute at most)
            sleep(WATCH_POLL)
        return None
    finally:
        lock.close()
