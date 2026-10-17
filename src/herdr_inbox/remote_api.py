"""The remote side of remote machines: ``inbox export`` and ``inbox apply`` (docs/REMOTE.md).

Both run on the machine that owns the database (over ssh, started by the operator's
machine with a FIXED command line); their request is JSON on stdin and their reply one JSON
object on stdout. ``export`` makes no data writes; ``apply`` records each request id in the
same transaction as its action, so a retried request is answered from the record.
"""

from __future__ import annotations

import hashlib
import json
import uuid
from datetime import timedelta

from . import service
from .store import Question, SchemaNotReady, Store, VIEW, now

EXPORT_VERSION = 1
RECENT = timedelta(days=7)
ACTIONS = ("answer", "dismiss", "hide", "deliver", "ack_alert")


def db_id(store: Store) -> str:
    """This database's random identity (created with it; a recreated database has a new one)."""
    row = store.db.execute("SELECT value FROM meta WHERE key = 'db_id'").fetchone()
    if row:
        return row[0]
    store.db.execute("INSERT OR IGNORE INTO meta (key, value) VALUES ('db_id', ?)", (uuid.uuid4().hex,))
    return store.db.execute("SELECT value FROM meta WHERE key = 'db_id'").fetchone()[0]


def qkey(store: Store, q: Question, dbid: str | None = None) -> str:
    """A question's identity beyond its number."""
    text = f"{dbid or db_id(store)}|{q.created_at}|{q.norm}|{q.session_id or ''}"
    return hashlib.sha256(text.encode()).hexdigest()[:32]


def _ext(store: Store, qid: int) -> dict:
    row = store.db.execute("SELECT hidden_at, agent_gone_at FROM question_ext WHERE id = ?", (qid,)).fetchone()
    return {"hidden": bool(row and row[0]), "agent_gone": bool(row and row[1])}


def _row(store: Store, q: Question, dbid: str) -> dict:
    return {
        "id": q.id, "qkey": qkey(store, q, dbid), "group": q.group, "status": q.status,
        "question": q.question, "header": q.header, "options": q.options or [],
        "multi_select": q.multi_select, "tab_label": q.tab_label, "pane_id": q.pane_id,
        "asked_by": q.asked_by, "title": q.group_title, "created_at": q.created_at,
        "answered_at": q.answered_at, "answer": q.answer, "delivery_error": q.delivery_error,
        "delivered_at": q.delivered_at, "verified_at": q.verified_at,
        "acknowledged_at": q.acknowledged_at, "delivery_attempts": q.delivery_attempts, **_ext(store, q.id),
    }


def export(store: Store, request: dict) -> dict:
    """Questions changed since the cursor ``{db_id, seq}``; a full snapshot (with the ids of
    every live question) when there is no usable cursor or ``full`` is asked for."""
    from . import __version__

    dbid = db_id(store)
    seq = store.db.execute("SELECT COALESCE(MAX(seq), 0) FROM changes").fetchone()[0]
    oldest = store.db.execute("SELECT COALESCE(MIN(seq), 0) FROM changes").fetchone()[0]
    cursor = request.get("cursor") if isinstance(request.get("cursor"), dict) else None
    full = (bool(request.get("full")) or cursor is None or cursor.get("db_id") != dbid
            or not isinstance(cursor.get("seq"), int) or cursor["seq"] < oldest - 1)
    if full:
        rows = store.db.execute(
            f"SELECT * FROM {VIEW} WHERE status = 'open'"
            " OR (status = 'answered' AND acknowledged_at IS NULL AND answered_at >= ?) ORDER BY id",
            (now(-RECENT),)).fetchall()
        qs = [Question.from_row(r) for r in rows]
    else:
        ids = [r[0] for r in store.db.execute(
            "SELECT DISTINCT qid FROM changes WHERE seq > ? ORDER BY qid", (cursor["seq"],))]
        got = {i: store.get(i) for i in ids}
        qs = [q for q in got.values() if q is not None]
        gone = [i for i, q in got.items() if q is None]
    out = {"version": __version__, "format": EXPORT_VERSION, "db_id": dbid, "seq": seq, "full": full,
           "questions": [_row(store, q, dbid) for q in qs], "alerts": _alerts(store, dbid)}
    if full:
        out["ids"] = [q.id for q in qs]
    else:
        out["gone"] = gone  # changed and no longer there (deleted)
    return out


def _alerts(store: Store, dbid: str) -> list[dict]:
    """Open alerts, plus those closed in the last hour (so the mirror sees them close). Small:
    sent whole on every pull."""
    from . import alerts

    rows = store.db.execute("SELECT * FROM alerts WHERE status = 'open' OR COALESCE(acked_at, closed_at) >= ?"
                            " ORDER BY id LIMIT 200", (now(-timedelta(hours=1)),)).fetchall()
    out = []
    for r in rows:
        a = alerts.Alert(**dict(r))
        out.append({"id": a.id, "akey": alerts.akey(store, a, dbid), "kind": a.kind, "severity": a.severity,
                    "title": a.title, "message": a.message, "tab_label": a.tab_label, "pane_id": a.pane_id,
                    "asked_by": a.asked_by, "status": a.status, "created_at": a.created_at})
    return out


def spawn_delivery(qids: list[int], force: bool = False) -> None:
    """Deliver detached (the ssh call returns at once; the remote worker retries)."""
    service.spawn_cli(["deliver", *map(str, qids)] + (["--force"] if force else []))


def _record(store: Store, request_id: str, qid, outcome: str, reason: str | None = None) -> dict:
    store.db.execute("INSERT INTO applied_requests (request_id, qid, outcome, reason, at) VALUES (?, ?, ?, ?, ?)",
                     (request_id, qid, outcome, reason, now()))
    return {"outcome": outcome, "reason": reason} if reason else {"outcome": outcome}


def _ack_alert(store: Store, request_id: str, request: dict) -> dict:
    """The operator acknowledged a remote alert: same request_id idempotence as answers."""
    from . import alerts

    qid = request.get("id")
    with store.transaction():
        done = store.db.execute("SELECT outcome, reason FROM applied_requests WHERE request_id = ?",
                                (request_id,)).fetchone()
        if done:
            return {"outcome": "already_applied"} if done[0] == "applied" else \
                {"outcome": done[0], "reason": done[1]}
        if request.get("db_id") != db_id(store):
            return _record(store, request_id, qid, "rejected", "reset")
        a = alerts.get(store, qid) if isinstance(qid, int) else None
        if a is None or request.get("qkey") != alerts.akey(store, a, db_id(store)):
            return _record(store, request_id, qid, "rejected", "not that alert")
        if a.status != "open":
            return _record(store, request_id, qid, "rejected", a.status)
        store.db.execute("UPDATE alerts SET status = 'acked', acked_at = ? WHERE id = ?", (now(), qid))
        result = _record(store, request_id, qid, "applied")
    service.update_markers(store, a.pane_id)
    return result


def apply(store: Store, request: dict) -> dict:
    """One operator action on one question, idempotent by ``request_id``."""
    request_id, action = request.get("request_id"), request.get("action")
    if action == "status" and isinstance(request_id, str) and 1 <= len(request_id) <= 64:
        # asked after an expiry, instead of a late send: did this request apply? If not, it
        # never may: a tombstone, in the same locked transaction, so the original apply (still
        # running after the caller's ssh was killed) refuses if it commits later
        with store.transaction():
            done = store.db.execute("SELECT outcome, reason FROM applied_requests WHERE request_id = ?",
                                    (request_id,)).fetchone()
            if done:
                return {"outcome": "recorded", "state": done[0], "reason": done[1]}
            _record(store, request_id, None, "rejected", "expired")
            return {"outcome": "expired"}
    if not isinstance(request_id, str) or not 1 <= len(request_id) <= 64 or action not in ACTIONS:
        return {"outcome": "rejected", "reason": "bad request"}
    if action == "ack_alert":
        return _ack_alert(store, request_id, request)
    after = None
    with store.transaction():  # the record and the action commit together
        done = store.db.execute("SELECT outcome, reason FROM applied_requests WHERE request_id = ?",
                                (request_id,)).fetchone()
        if done:
            return {"outcome": "already_applied"} if done[0] == "applied" else \
                {"outcome": done[0], "reason": done[1]}
        qid = request.get("id")
        if request.get("db_id") != db_id(store):
            return _record(store, request_id, qid, "rejected", "reset")
        q = store.get(qid) if isinstance(qid, int) else None
        if q is None or request.get("qkey") != qkey(store, q):
            return _record(store, request_id, qid, "rejected", "not that question")
        if action == "answer":
            detail = request.get("detail") if isinstance(request.get("detail"), dict) else {}
            d = service.make_detail(detail.get("text"), detail.get("choices"), detail.get("notes"))
            if d is None:
                return _record(store, request_id, qid, "rejected", "empty answer")
            text = service.format_answer(d.get("choices") or [], d.get("notes"), d.get("text"))
            if q.status == "answered":
                if q.answer == text:  # ours, applied before a lost reply / a crash
                    _record(store, request_id, qid, "applied")
                    return {"outcome": "already_applied"}
                return _record(store, request_id, qid, "rejected", f"answered with {q.answer}")
            if q.status != "open":
                return _record(store, request_id, qid, "rejected", q.status)
            store._set_answer(qid, text, d)
            after = ("deliver", False)
        elif action == "dismiss":
            if q.status != "open":
                return _record(store, request_id, qid, "rejected", q.status)
            store.set_status(qid, "dismissed")
            after = ("markers", None)
        elif action == "hide":
            store.hide_failed(qid)
        elif action == "deliver":
            if q.status != "answered" or q.delivered_at:
                return _record(store, request_id, qid, "rejected", "nothing to deliver")
            after = ("deliver", True)
        result = _record(store, request_id, qid, "applied")
    if after and after[0] == "deliver":
        spawn_delivery([qid], force=after[1])
        service.update_markers(store, q.pane_id)  # the answered question leaves the tab's ❓N
    elif after and after[0] == "markers":
        service.update_markers(store, q.pane_id)
    return result


def run_cli(command: str, raw: str) -> tuple[int, dict]:
    try:
        request = json.loads(raw)
        if not isinstance(request, dict):
            raise ValueError("not an object")
    except ValueError as exc:
        return 1, {"error": f"bad request: {exc}"}
    try:  # never migrates (store.py): this machine's worker does
        if command == "export":
            with Store(timeout=5, maintenance=False) as store:  # no data writes from a pull
                return 0, export(store, request)
        with Store(timeout=5, maintenance=False) as store:
            return 0, apply(store, request)
    except SchemaNotReady as exc:
        service._ensure_worker()  # this machine's worker migrates; the next pull gets through
        return 1, {"error": f"not ready: {exc}"}
