"""The remote-machine rows of the popup transition golden (P6-D2): what
``tools/capture_popup_transitions.py`` adds for them. The popup reads remote machines from the
local mirror only (``remote_machines``, ``remote_questions``, ``remote_alerts``,
``remote_outbox``), so a scenario writes those tables directly; nothing reaches a network.

- ``snapshot(store)``: the snapshot's ``remote`` part, or None when the mirror is empty (the
  scenarios without remote machines stay byte-identical);
- ``install(rec)``: the popup's remote writes recorded as effects (``remote_answer``,
  ``remote_dismiss``, ``remote_ack``, ``clear_outbox``, ``clear_problem``), after the real write;
- ``reset()``: request ids (``remote_outbox.request_id``) counted from 1 again, and the mirror's
  clock held, per scenario: the golden never holds a random id or the real time;
- the setup helpers the remote scenarios use (``machine``, ``rq``, ``ralert``, ``outbox``).

The format is in tests/fixtures/README.md ("popup-transitions.json", remote machines).
"""

from __future__ import annotations

import json
import types
from datetime import datetime, timedelta, timezone

HELD = datetime(2026, 10, 17, 12, 0, 0, tzinfo=timezone.utc)  # the mirror's clock (remote._now)
MID, LABEL, DBID = "m1", "project1", "d" * 32
_TABLES = ("remote_machines", "remote_questions", "remote_alerts", "remote_outbox")
_ids = {"n": 0}


def reset() -> None:
    _ids["n"] = 0


def _uuid4():
    _ids["n"] += 1
    return types.SimpleNamespace(hex=f"{_ids['n']:032x}")


def _rq_json(d: dict) -> dict:
    opts = d.get("options") or []
    return {"id": d["id"], "group": d["group"], "status": d["status"], "pane_id": d.get("pane_id"),
            "options": [o["label"] for o in opts], "multi_select": bool(d.get("multi_select")),
            "has_preview": any(o.get("preview") for o in opts), "db_id": d["_db_id"], "qkey": d.get("qkey")}


def snapshot(store) -> dict | None:
    """The mirror as the popup reads it (``remote.open_groups``, ``open_alerts``, ``outbox_rows``,
    ``problem_rows``), with the identities the popup's writes check (``mirror``)."""
    from herdr_inbox import remote

    if not any(store.db.execute(f"SELECT 1 FROM {t} LIMIT 1").fetchone() for t in _TABLES):
        return None
    db = store.db
    return {
        "machines": [{"machine_id": r[0], "label": r[1]}
                     for r in db.execute("SELECT machine_id, label FROM remote_machines ORDER BY machine_id")],
        "groups": [{"key": k, "machine_id": m["machine_id"], "questions": [_rq_json(d) for d in qs]}
                   for k, m, qs in remote.open_groups(store)],
        "mirror": [{"machine_id": r[0], "remote_id": r[1], "db_id": r[2], "qkey": r[3]} for r in db.execute(
            "SELECT machine_id, remote_id, db_id, qkey FROM remote_questions ORDER BY machine_id, remote_id")],
        "alerts": [{"machine_id": m["id"], "id": a["id"], "kind": a.get("kind"), "status": a.get("status"),
                    "pane_id": a.get("pane_id"), "db_id": a["_db_id"], "akey": a.get("akey")}
                   for m, a in remote.open_alerts(store)],
        "outbox": [{"request_id": r["request_id"], "machine_id": r["machine_id"], "action": r["action"],
                    "remote_id": r["remote_id"], "state": r["state"],
                    "clearable": remote.clearable(store, r["request_id"])} for r in remote.outbox_rows(store)],
        "problems": [{"machine_id": p["machine_id"], "remote_id": p["remote_id"]} for p in remote.problem_rows(store)],
    }


def hold() -> None:
    """The mirror's clock held at HELD, request ids counted (``reset`` restarts them)."""
    from herdr_inbox import remote

    remote._uuid = types.SimpleNamespace(uuid4=_uuid4)
    remote._now = lambda: HELD


def install(rec) -> None:
    """Record the popup's remote writes (each after the real one: a refused write records nothing)."""
    from herdr_inbox import remote

    hold()
    real_enqueue, real_ack = remote.enqueue, remote.enqueue_alert_ack
    real_settle, real_seen, real_clear = remote._settle, remote.mark_seen, remote.clear_problem
    expiring: set[str] = set()

    def ident(machine_id, remote_id, data) -> dict:
        return {"machine_id": machine_id, "remote_id": remote_id, "db_id": data.get("_db_id"), "qkey": data.get("qkey")}

    def enqueue(store, machine_id, data, action, detail=None):
        out = real_enqueue(store, machine_id, data, action, detail)
        if action == "answer":
            rec.add({"remote_answer": {**ident(machine_id, data["id"], data), "detail": detail}})
        else:
            rec.add({f"remote_{action}": ident(machine_id, data["id"], data)})
        return out

    def enqueue_alert_ack(store, machine_id, data):
        out = real_ack(store, machine_id, data)
        rec.add({"remote_ack": {"machine_id": machine_id, "alert_id": data["id"], "db_id": data.get("_db_id"),
                                "akey": data.get("akey")}})
        return out

    def settle(store, request_id, state, result, title=None, body=""):
        expiring.add(request_id)  # the popup's `d` on an expired row: settled, then marked seen
        return real_settle(store, request_id, state, result, title, body)

    def mark_seen(store, request_id):
        rec.add({"clear_outbox": {"request_id": request_id, "expire": request_id in expiring}})
        expiring.discard(request_id)
        return real_seen(store, request_id)

    def clear_problem(store, machine_id, remote_id):
        rec.add({"clear_problem": {"machine_id": machine_id, "remote_id": remote_id}})
        return real_clear(store, machine_id, remote_id)

    remote.enqueue, remote.enqueue_alert_ack = enqueue, enqueue_alert_ack
    remote._settle, remote.mark_seen, remote.clear_problem = settle, mark_seen, clear_problem


# -- setup helpers ---------------------------------------------------------------------------

def _iso(t: datetime) -> str:
    return t.isoformat(timespec="seconds")


def machine(store, mid: str = MID, label: str | None = LABEL, offline: bool = False) -> None:
    store.db.execute("INSERT INTO remote_machines (machine_id, label, target, db_id, last_ok_at, last_error)"
                     " VALUES (?, ?, ?, ?, ?, ?)", (mid, label, label or mid, DBID, _iso(HELD - timedelta(hours=2)),
                                                    "No route to host" if offline else None))


def rq(store, rid: int, question: str, *options: str, group: int | None = None, status: str = "open",
       mid: str = MID, dbid: str = DBID, qkey: str | None = None, **extra) -> dict:
    """A mirrored remote question (the export's cleaned row), asked from tab-A on the remote."""
    data = {"id": rid, "group": rid if group is None else group, "status": status, "question": question,
            "options": [{"label": o} for o in options], "multi_select": False, "hidden": False,
            "agent_gone": False, "delivery_attempts": 0, "qkey": qkey or f"{rid:02d}" * 8, "tab_label": "tab-A",
            "pane_id": "w1:p1", "asked_by": None, "header": None, "title": None,
            "created_at": _iso(HELD - timedelta(minutes=5 * rid)), **extra}
    store.db.execute("INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json,"
                     " first_seen_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(machine_id, remote_id) DO UPDATE SET"
                     " db_id = excluded.db_id, qkey = excluded.qkey, status = excluded.status,"
                     " data_json = excluded.data_json",
                     (mid, dbid, rid, data["qkey"], status, json.dumps(data), _iso(HELD)))
    return data


def ralert(store, aid: int, title: str, kind: str = "agent", status: str = "open", mid: str = MID) -> None:
    data = {"id": aid, "akey": f"a{aid}" * 8, "status": status, "kind": kind, "severity": "warning", "title": title,
            "message": None, "tab_label": "tab-A", "pane_id": "w1:p1", "asked_by": None,
            "created_at": _iso(HELD - timedelta(hours=aid))}
    store.db.execute("INSERT INTO remote_alerts (machine_id, db_id, alert_id, akey, status, data_json, first_seen_at)"
                     " VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(machine_id, alert_id) DO UPDATE SET"
                     " status = excluded.status, data_json = excluded.data_json",
                     (mid, DBID, aid, data["akey"], status, json.dumps(data), _iso(HELD)))


def shown(store, rid: int, mid: str = MID) -> dict:
    row = store.db.execute("SELECT db_id, data_json FROM remote_questions WHERE machine_id = ? AND remote_id = ?",
                           (mid, rid)).fetchone()
    return {**json.loads(row[1]), "_db_id": row[0]}


def outbox(store, rid: int, action: str, state: str = "queued", result: str | None = None,
           expired: bool = False, sending: bool = False, mid: str = MID) -> str:
    """An action on mirrored question ``rid`` in the outbox (queued by an earlier popup)."""
    from herdr_inbox import remote

    request_id = remote.enqueue(store, mid, shown(store, rid, mid), action,
                                {"choices": ["yes"]} if action == "answer" else None)
    store.db.execute("UPDATE remote_outbox SET state = ?, result = ?, expires_at = ?, sending_at = ?"
                     " WHERE request_id = ?",
                     (state, result, _iso(HELD - timedelta(minutes=1)) if expired else _iso(HELD + timedelta(hours=12)),
                      _iso(HELD) if sending else None, request_id))
    return request_id
