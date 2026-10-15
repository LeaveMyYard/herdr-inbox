"""Open the popup on a specific question or group (the integration API for other plugins).

``herdr plugin pane open`` cannot pass arguments to the pane command, so ``inbox open
--question Qn | --group G | --pane P | --session S`` writes a one-shot *focus request* next
to the database and then opens the popup; the popup consumes it on start:

- the request is ``{kind, value, created_at, nonce}``, written atomically (temp file +
  ``os.replace``); the newest request wins;
- the next popup start takes it with an atomic rename (two popups starting at once cannot
  both take it), deletes it, and honours it only if it is at most REQUEST_TTL old;
- the target resolves to a group (and for a question, that tab); a pane or a session means
  its oldest open group. A target that is gone falls back to the list with a notice.

The view it opens is like any view that appears: armed for ARM_SECONDS, cursor on a target.
"""

from __future__ import annotations

import contextlib
import fcntl
import json
import os
import re
import time
import uuid
from pathlib import Path

from .store import Question, Store, db_path

REQUEST_TTL = 5.0
KINDS = ("question", "group", "pane", "session")
NONCE_ENV = "INBOX_OPEN_NONCE"  # `inbox open` hands its request's nonce to the popup it opens
VALID = {"question": re.compile(r"[Qq]?\d+"), "group": re.compile(r"\d+"),
         "pane": re.compile(r"\S+"), "session": re.compile(r"\S+")}


def request_path() -> Path:
    return db_path().parent / "open-request.json"


def valid(kind: str, value: str) -> bool:
    return kind in KINDS and bool(VALID[kind].fullmatch(str(value)))


def write_request(kind: str, value: str, hold: bool = True) -> str:
    """Write the request; returns its nonce (passed to the popup as INBOX_OPEN_NONCE).
    ``hold=False`` (``inbox open --no-hold``, the caller's choice, only for an explicit
    operator key/click): the popup opens like a normal one, not held until Tab/→."""
    if not valid(kind, value):
        raise ValueError(f"{kind} {value!r}")
    path = request_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    data = {"kind": kind, "value": str(value), "created_at": time.time(), "nonce": uuid.uuid4().hex}
    if not hold:
        data["hold"] = False
    tmp = path.with_name(f".open-request-{os.getpid()}-{data['nonce'][:8]}")
    tmp.write_text(json.dumps(data))
    os.replace(tmp, path)
    return data["nonce"]


def consume_request() -> dict | None:
    """Take the pending request, if this popup was opened for it (its INBOX_OPEN_NONCE),
    once, whoever asks first; None if there is none, it is someone else's, stale or
    unreadable. A popup opened by hand never takes a request."""
    nonce = os.environ.get(NONCE_ENV)
    path = request_path()
    if not nonce or not path.exists():
        return None
    try:
        if json.loads(path.read_text()).get("nonce") != nonce:
            return None  # another `inbox open`'s request (or a newer one): not ours
    except (OSError, ValueError):
        pass  # unreadable: take it below, and drop it
    taken = path.with_name(f".open-request-taken-{os.getpid()}-{uuid.uuid4().hex[:8]}")
    try:
        os.rename(path, taken)
    except OSError:
        return None
    try:
        data = json.loads(taken.read_text())
    except (OSError, ValueError):
        data = None
    finally:
        try:
            taken.unlink()
        except OSError:
            pass
    if not isinstance(data, dict) or data.get("kind") not in KINDS or data.get("nonce") != nonce:
        return None
    age = time.time() - float(data.get("created_at") or 0)
    if not 0 <= age <= REQUEST_TTL:
        return None
    request = {"kind": data["kind"], "value": str(data.get("value", ""))}
    if data.get("hold") is False:  # only an explicit false lifts the hold
        request["hold"] = False
    return request


def _oldest_group(qs: list[Question]) -> tuple[tuple[str, int], None] | None:
    if not qs:
        return None
    return ("group", min(qs, key=lambda q: q.id).group), None


def resolve(store: Store, target: dict) -> tuple[tuple[str, int], int | None] | None:
    """(group entry, question id to show or None) for an open target; None if it is gone."""
    kind, value = target.get("kind"), str(target.get("value", "")).strip()
    open_qs = store.list()
    if kind in ("question", "group"):
        try:
            n = int(value.lstrip("Qq") if kind == "question" else value)
        except ValueError:
            return None
        if kind == "question":
            q = next((q for q in open_qs if q.id == n), None)
            return (("group", q.group), q.id) if q else None
        return (("group", n), None) if any(q.group == n for q in open_qs) else None
    if kind == "pane":
        return _oldest_group([q for q in open_qs if q.pane_id == value])
    if kind == "session":
        return _oldest_group([q for q in open_qs if q.session_id == value])
    return None


def describe(target: dict) -> str:
    kind, value = target.get("kind"), target.get("value")
    if kind == "question":
        return f"Q{str(value).lstrip('Qq')}"
    return f"{kind} {value}"


def _lock_path() -> Path:
    return db_path().parent / "popup.lock"


@contextlib.contextmanager
def popup_running():
    """Held by a running popup for its lifetime (exclusive flock), so `inbox open` can tell
    the caller that the popup is already open instead of leaving a request nobody takes."""
    path = _lock_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    fh = open(path, "a")
    try:
        try:
            fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            pass  # another popup holds it; this one still runs
        yield
    finally:
        fh.close()


def popup_is_running() -> bool:
    try:
        fh = open(_lock_path(), "a")
    except OSError:
        return False
    try:
        fcntl.flock(fh, fcntl.LOCK_SH | fcntl.LOCK_NB)
        return False
    except OSError:
        return True
    finally:
        fh.close()


def hold_popup_lock():
    """The running popup's lock (see popup_running); close the returned file to release."""
    path = _lock_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    fh = open(path, "a")
    try:
        fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        pass
    return fh
