"""Alerts (0.8): "please check X" from an agent, with no answer expected; and automatic
alerts for agents stuck at an approval or question prompt.

- ``raise_alert`` (the ``alert_operator`` MCP tool): title, message, severity
  (info | warning | urgent). One herdr notification; it counts in the pane's ❓N badge and
  ``$inbox_q``; the popup lists it until the operator acknowledges it (or the agent
  withdraws it). Nothing is ever typed back: the agent sees the acknowledgement in
  ``check_answers`` / ``list_my_questions``.
- ``scan_blocked`` (the delivery worker, every tick): an agent herdr reports ``blocked``
  (it recognised an approval or question UI -- the agent cannot call any tool then) for
  BLOCKED_AFTER seconds gets an automatic alert "<tab> is waiting for you", cleared when the
  status changes. Nothing is ever typed into such a pane.
"""

from __future__ import annotations

import hashlib
import time
from dataclasses import dataclass
from datetime import datetime, timezone

from . import herdr, service
from .store import Store, now

SEVERITIES = ("info", "warning", "urgent")
MAX_TITLE, MAX_MESSAGE = 120, 2000
BLOCKED_AFTER = 15.0  # a permission prompt answered within this never raises an alert


@dataclass
class Alert:
    id: int
    created_at: str
    kind: str
    severity: str
    title: str
    message: str | None
    pane_id: str | None
    tab_id: str | None
    tab_label: str | None
    session_id: str | None
    asked_by: str | None
    herdr_server: str | None
    status: str
    acked_at: str | None
    closed_at: str | None
    reported_at: str | None

    @property
    def ref(self) -> str:
        return f"A{self.id}"


def _get(store: Store, aid: int) -> Alert | None:
    row = store.db.execute("SELECT * FROM alerts WHERE id = ?", (aid,)).fetchone()
    return Alert(**dict(row)) if row else None


def get(store: Store, aid: int) -> Alert | None:
    return _get(store, aid)


def open_alerts(store: Store) -> list[Alert]:
    return [Alert(**dict(r)) for r in store.db.execute(
        "SELECT * FROM alerts WHERE status = 'open' ORDER BY"
        " CASE severity WHEN 'urgent' THEN 0 WHEN 'warning' THEN 1 ELSE 2 END, id").fetchall()]


def akey(store: Store, a: Alert, db_id: str) -> str:
    return hashlib.sha256(f"{db_id}|alert|{a.id}|{a.created_at}|{a.title}".encode()).hexdigest()[:32]


def raise_alert(store: Store, caller, title: str, message: str | None = None, severity: str = "warning",
                asked_by: str | None = None) -> dict:
    if not caller.pane_id:
        return dict(service.UNAVAILABLE)
    severity = severity if severity in SEVERITIES else "warning"
    title = " ".join(str(title or "").split())[:MAX_TITLE]
    if not title:
        return {"status": "error", "message": "An alert needs a short title."}
    message = (str(message).strip()[:MAX_MESSAGE] or None) if message else None
    with store.transaction():
        dup = store.db.execute("SELECT id FROM alerts WHERE status = 'open' AND kind = 'agent' AND title = ?"
                               " AND COALESCE(session_id, pane_id) = COALESCE(?, ?)",
                               (title, caller.session_id, caller.pane_id)).fetchone()
        if dup:
            return {"id": dup[0], "status": "already_open",
                    "message": f"A{dup[0]} is already with the operator; do not raise it again."}
        cur = store.db.execute(
            "INSERT INTO alerts (created_at, kind, severity, title, message, pane_id, tab_id, tab_label, session_id,"
            " asked_by, herdr_server) VALUES (?, 'agent', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (now(), severity, title, message, caller.pane_id, caller.tab_id, caller.tab_label, caller.session_id,
             asked_by, herdr.server_id()))
        aid = cur.lastrowid
    where = caller.tab_label or caller.pane_id
    service.later(lambda: herdr.notify(f"A{aid} · {severity} · {where}", title))
    service.update_markers(store, caller.pane_id)
    return {"id": aid, "status": "open", "message": (
        f"Raised as A{aid}. The operator sees it in the inbox and will check; no answer comes back. "
        f"check_answers reports when it is acknowledged. Withdraw it (withdraw_alert) if it resolves itself.")}


def _close(store: Store, aid: int, status: str) -> Alert | None:
    a = _get(store, aid)
    if a is None or a.status != "open":
        return None
    column = "acked_at" if status == "acked" else "closed_at"
    if store.db.execute(f"UPDATE alerts SET status = ?, {column} = ? WHERE id = ? AND status = 'open'",
                        (status, now(), aid)).rowcount != 1:
        return None
    service.update_markers(store, a.pane_id)
    return a


def acknowledge(store: Store, aid: int) -> bool:
    return _close(store, aid, "acked") is not None


def withdraw(store: Store, caller, aid: int) -> dict:
    a = _get(store, aid)
    if a is None or a.kind != "agent" or (a.session_id or a.pane_id) != (caller.session_id or caller.pane_id):
        return {"id": aid, "status": "error", "message": f"A{aid} is not an alert of this session."}
    if _close(store, aid, "withdrawn") is None:
        return {"id": aid, "status": "error", "message": f"A{aid} is {a.status}, not open."}
    return {"id": aid, "status": "withdrawn", "message": f"A{aid} withdrawn."}


def acknowledged_unreported(store: Store, caller) -> list[Alert]:
    """This session's alerts the operator acknowledged since the last report (check_answers)."""
    rows = store.db.execute(
        "SELECT * FROM alerts WHERE kind = 'agent' AND status = 'acked' AND reported_at IS NULL"
        " AND COALESCE(session_id, pane_id) = COALESCE(?, ?) ORDER BY id",
        (caller.session_id, caller.pane_id)).fetchall()
    out = [Alert(**dict(r)) for r in rows]
    for a in out:
        store.db.execute("UPDATE alerts SET reported_at = ? WHERE id = ?", (now(), a.id))
    return out


def mine(store: Store, caller) -> list[Alert]:
    return [Alert(**dict(r)) for r in store.db.execute(
        "SELECT * FROM alerts WHERE kind = 'agent' AND COALESCE(session_id, pane_id) = COALESCE(?, ?) ORDER BY id",
        (caller.session_id, caller.pane_id)).fetchall()]


# -- automatic alerts for agents at a blocking prompt ---------------------------------------

CLEAR_AFTER = 10.0  # a pane counts as unblocked after this long AND two ticks: no flapping
# When each pane's current prompt was first seen lives in the table `blocked_panes`, so a
# worker restart or hand-over does not restart the BLOCKED_AFTER clock. The hysteresis
# memory below may reset with the process: that only delays a clear.
_unblocked: dict[tuple[str, str], tuple[float, int]] = {}  # (server, pane) -> (first non-blocked sighting, ticks)


def _age(iso: str) -> float:
    try:
        return (datetime.now(timezone.utc) - datetime.fromisoformat(iso)).total_seconds()
    except ValueError:
        return 0.0


def scan_blocked(store: Store) -> int:
    """One `herdr agent list`: agents `blocked` for BLOCKED_AFTER seconds get an alert;
    alerts of agents no longer blocked are cleared. Never types anything anywhere.
    Returns how many agents herdr reports (the worker stays alive while there are any).

    Hysteresis: a pane leaves its prompt only after CLEAR_AFTER seconds and two ticks of
    another status (herdr's detection can flicker within one prompt); a pane whose agent
    is gone from the list leaves at once."""
    res, _ = herdr.call("agent", "list")
    if res is None:
        return 0
    agents = [a for a in (res or {}).get("agents", []) if isinstance(a, dict) and a.get("pane_id")]
    present = {a["pane_id"] for a in agents}
    blocked = {a["pane_id"]: a for a in agents if a.get("agent_status") == "blocked"}
    t = time.monotonic()
    server = herdr.server_id()
    since = {r[0]: r[1] for r in store.db.execute(
        "SELECT pane_id, since FROM blocked_panes WHERE herdr_server = ?", (server,)).fetchall()}
    for pane in list(since):
        if pane in blocked:
            _unblocked.pop((server, pane), None)
            continue
        first, ticks = _unblocked.get((server, pane), (t, 0))
        _unblocked[(server, pane)] = (first, ticks + 1)
        if pane not in present or (ticks + 1 >= 2 and t - first >= CLEAR_AFTER):
            store.db.execute("DELETE FROM blocked_panes WHERE herdr_server = ? AND pane_id = ?", (server, pane))
            del since[pane]
            _unblocked.pop((server, pane), None)
    for pane in blocked:
        if pane not in since:
            since[pane] = now()
            store.db.execute("INSERT OR IGNORE INTO blocked_panes (herdr_server, pane_id, since) VALUES (?, ?, ?)",
                             (server, pane, since[pane]))
    waiting = set(since)  # blocked now, or within the hysteresis
    scope = " AND COALESCE(herdr_server, ?) = ?"
    open_auto = {r[0]: r[1] for r in store.db.execute(
        "SELECT pane_id, id FROM alerts WHERE status = 'open' AND kind = 'blocked'" + scope, (server, server)).fetchall()}
    for pane, aid in open_auto.items():
        if pane not in waiting:
            _close(store, aid, "cleared")
    # a dismissed (acked) blocked alert holds until that pane is seen not blocked: closed_at
    # marks the end of its episode, so the next prompt alerts again (survives worker restarts)
    marks = ",".join("?" * len(waiting))
    store.db.execute("UPDATE alerts SET closed_at = ? WHERE kind = 'blocked' AND status = 'acked'"
                     " AND closed_at IS NULL" + scope + (f" AND pane_id NOT IN ({marks})" if waiting else ""),
                     (now(), server, server, *sorted(waiting)))
    dismissed = {r[0] for r in store.db.execute(
        "SELECT pane_id FROM alerts WHERE kind = 'blocked' AND status = 'acked' AND closed_at IS NULL" + scope,
        (server, server))}
    for pane, agent in blocked.items():
        if pane in open_auto or pane in dismissed or _age(since[pane]) < BLOCKED_AFTER:
            continue
        tab_label = None
        if agent.get("tab_id"):
            label = herdr.tab_label(agent["tab_id"])
            from . import tabs

            tab_label = tabs.base_label(label, {tabs.DEFAULT_PREFIX}) if label else None
        where = tab_label or pane
        title = f"{where} is waiting for you"
        with store.transaction():
            if store.db.execute("SELECT 1 FROM alerts WHERE status = 'open' AND kind = 'blocked' AND pane_id = ?"
                                + scope, (pane, server, server)).fetchone():
                continue
            cur = store.db.execute(
                "INSERT INTO alerts (created_at, kind, severity, title, message, pane_id, tab_id, tab_label, session_id,"
                " asked_by, herdr_server) VALUES (?, 'blocked', 'warning', ?, ?, ?, ?, ?, ?, ?, ?)",
                (now(), title, "The agent is at an approval or question prompt and cannot continue until you answer it there.",
                 pane, agent.get("tab_id"), tab_label, (agent.get("agent_session") or {}).get("value"),
                 agent.get("agent"), server))
        herdr.notify(f"A{cur.lastrowid} · {where}", "waiting for you at a prompt")
        service.update_markers(store, pane)
    return len(agents)
