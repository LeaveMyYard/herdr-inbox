"""Keystroke-free delivery to an IDLE Claude session (0.10.0): the asyncRewake watcher.

Claude Code runs a command hook marked ``asyncRewake: true`` in the background; when it
exits 2, its stderr (or, if empty, its stdout) wakes the session as a system reminder and
starts a turn (probed on 2.1.301: an idle session answered 2 s after the exit). So every
``Stop`` arms one watcher for its session (``run``):

- it waits for an answer for this session that is neither delivered nor seen, polling the
  database read-only (``WATCH_POLL``);
- only while herdr reports the pane's agent ``idle``/``done`` with this very session (a
  working session gets it from the PostToolBatch/Stop hooks; a blocked one must not be
  disturbed; a pane holding another conversation ends the watch);
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

WATCH_POLL = 1.0
WATCH_MAX_SECONDS = 12 * 3600.0 - 300  # under the hooks.json timeout (43200 s), which Claude Code enforces
LOCK_WAIT = 5.0  # for the superseded watcher to let go
WAKE_EXIT = 3  # run.py's private "wake" code; the hooks.json wrapper maps it (and only it) to 2


def _dir() -> Path:
    from .store import db_path

    d = db_path().parent / "rewake"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _safe(session_id: str) -> str:
    return "".join(c for c in session_id if c.isalnum() or c in "-_")[:80]


def watcher_alive(session_id: str | None) -> bool:
    """Is a watcher for ``session_id`` alive (holding its lock)?"""
    if not session_id:
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
    """'go' (idle/done, this session), 'wait' (working, blocked, unknown), or 'end'."""
    from . import herdr

    agent = herdr.live_agent(pane)
    if agent is None or (agent.get("session_id") and agent["session_id"] != session_id):
        return "end"  # agent gone, or the pane holds another conversation now
    return "go" if agent.get("status") in ("idle", "done") else "wait"


def run(payload: dict, clock=time.monotonic, sleep=time.sleep) -> str | None:
    """Watch for this session's answers; the wake text, or None (nothing to wake for)."""
    from . import hook, proc

    session = payload.get("session_id")
    pane = os.environ.get("HERDR_PANE_ID")
    if not session or not pane or payload.get("agent_id"):
        return None
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
        while clock() < end:
            if not _still_owner(session, token) or not proc.alive(owner):
                return None
            hook._touch(session)  # alive and watching: keeps the session's hook live (at most a write a minute)
            if hook._has_candidates(session):
                state = _ready(pane, session)
                if state == "end":
                    return None
                if state == "go":
                    text = hook.deliver(payload)
                    if text:
                        return text
            sleep(WATCH_POLL)
        return None
    finally:
        lock.close()
