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

import json
import os
import time
import uuid
from pathlib import Path

from .store import Question, Store, db_path

REQUEST_TTL = 5.0
KINDS = ("question", "group", "pane", "session")


def request_path() -> Path:
    return db_path().parent / "open-request.json"


def write_request(kind: str, value: str) -> None:
    if kind not in KINDS:
        raise ValueError(kind)
    path = request_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    data = {"kind": kind, "value": str(value), "created_at": time.time(), "nonce": uuid.uuid4().hex}
    tmp = path.with_name(f".open-request-{os.getpid()}-{data['nonce'][:8]}")
    tmp.write_text(json.dumps(data))
    os.replace(tmp, path)


def consume_request() -> dict | None:
    """Take the pending request (once, whoever asks first); None if there is none, it is
    stale or unreadable."""
    path = request_path()
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
    if not isinstance(data, dict) or data.get("kind") not in KINDS:
        return None
    age = time.time() - float(data.get("created_at") or 0)
    if not 0 <= age <= REQUEST_TTL:
        return None
    return {"kind": data["kind"], "value": str(data.get("value", ""))}


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
