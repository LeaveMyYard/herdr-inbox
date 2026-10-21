"""pytest plugin: record the delivery decision table (herdr-panorama docs/plans/rust-rewrite.md
§5.2 item 5) while the inbox tests run. Loaded only on request, never by a plain ``pytest``:

    uv run pytest -p tests.decision_recorder --decision-table OUT.json [paths]

It sets ``herdr_inbox.decision_trace.RECORDER`` for the run. Every traced decision (a pane of
``service.deliver_batch``, a ``hook.deliver`` firing, a ``service.verify_hook_deliveries`` or
``service.verify_job`` verification, a poll of the ``rewake`` watcher) becomes one row per
question: the inputs and observations the code looked up, in the order it looked, and the
question's delivery state before and after. Rows are normalised (no ids, times, paths or
claim tokens), deduplicated, and written in a stable order with the tests that drove each.
``tools/capture_goldens.py --only decisions`` runs this; tests/fixtures/README.md has the format.
"""

from __future__ import annotations

import hashlib
import json
import re
import sqlite3
import threading
from datetime import datetime, timedelta, timezone

import pytest

from herdr_inbox import decision_trace
from herdr_inbox.store import AGENT_GONE, CLAIM_STALE, PENDING

PANE = re.compile(r"\bw\d+:p\d+\b")
TIME = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[+-]\d{2}:\d{2}|Z)?")
PATH = re.compile(r"(?<![\w.])/[^\s'\"]+")
SESSION = re.compile(r"\b[0-9a-f]{8}(?:-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})?\b")  # an id or its prefix


def text(value: str) -> str:
    """Free text of a reason or an error, without what differs between runs or machines."""
    return SESSION.sub("<session>", PATH.sub("<path>", TIME.sub("<time>", PANE.sub("<pane>", value))))


def agent_view(agent, label=lambda session: None) -> dict | None:
    """What the policy reads from herdr's agent record: never its ids, only which session it
    runs, as a label of its own frame (``label``: s1, s2, ... in order of appearance), so a
    pane that switched conversations between two looks shows."""
    if agent is None:
        return None
    return {"kind": agent.get("agent"), "status": agent.get("status"), "focused": bool(agent.get("focused")),
            "session": label(agent.get("session_id"))}


def value_of(key: str, value):
    if key in ("agent", "agent_again"):
        return agent_view(value)
    if key == "landed":  # segment indexes of service.expected_segments (2: the long-answer reference)
        return None if value is None else sorted(value)
    if isinstance(value, str):
        return text(value)
    if isinstance(value, (set, frozenset, tuple, list)):
        return sorted(value)
    return value


STATE_SQL = ("SELECT q.status, q.pane_id, q.delivered_at, q.delivery_error, q.acknowledged_at, q.delivery_attempts,"
             " e.verified_at, e.hook_sent_at, e.hook_misses, e.busy_attempts, e.pending_since, e.agent_gone_at"
             " FROM questions q LEFT JOIN question_ext e ON e.id = q.id WHERE q.id = ?")


def state(path: str, qid: int) -> dict | None:
    """A question's delivery state (the DeliveryState of the plan's §2.3), read on a connection of
    its own (the store is autocommit; a hook's store may be closed by the time a frame ends)."""
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=5)
    try:
        row = db.execute(STATE_SQL, (qid,)).fetchone()
    finally:
        db.close()
    if row is None:
        return None
    (status, pane, delivered, error, acked, attempts, verified, hook_sent, hook_misses, busy_attempts,
     pending_since, gone_at) = row
    out: dict = {"status": status}
    error = error or ""
    if error.startswith("sending:"):
        out["state"] = "claimed"
        cutoff = f"sending:{(datetime.now(timezone.utc) - CLAIM_STALE).isoformat(timespec='seconds')}"
        out["claim_stale"] = error < cutoff
    elif delivered:
        out["state"] = "delivered"
    elif error.startswith(PENDING):
        out["state"], out["reason"] = "pending", text(error[len(PENDING):])
    elif error.startswith(AGENT_GONE):
        out["state"], out["reason"] = "agent_gone", text(error[len(AGENT_GONE):])
    elif error:
        out["state"], out["reason"] = "failed", text(error)
    else:
        out["state"] = "unattempted"
    flags = {"verified": verified, "acknowledged": acked, "hook_sent": hook_sent, "pending_since": pending_since,
             "agent_gone_marked": gone_at}
    out.update({k: True for k, v in flags.items() if v})
    out.update({k: v for k, v in (("attempts", attempts), ("hook_misses", hook_misses),
                                  ("busy_attempts", busy_attempts)) if v})
    out["_pane"] = pane
    return out


class Frame:
    def __init__(self, rec: "Recorder", path: str, inputs: dict):
        self.rec, self.path, self.inputs = rec, path, {k: value_of(k, v) for k, v in inputs.items()}
        self.obs: dict = {}
        self.qobs: dict[int, dict] = {}
        self.before: dict[int, dict | None] = {}
        self.db: str | None = None
        self.sessions: dict[str, str] = {}
        self.messages = 0  # deliver_batch's message batches, in typing order

    def label(self, session: str | None) -> str | None:
        """``session`` as this frame's s1, s2, ... (None when unknown)."""
        return self.sessions.setdefault(session, f"s{len(self.sessions) + 1}") if session else None

    def track(self, store, qs) -> None:
        if store is None:
            return
        self.db = str(store.path)
        for q in qs:
            qid = q if isinstance(q, int) else q.id
            if qid not in self.before:
                self.before[qid] = state(self.db, qid)
                self.qobs[qid] = {}

    def note(self, key: str, value, qids=None) -> None:
        v = agent_view(value, self.label) if key in ("agent", "agent_again") else value_of(key, value)
        if key == "full_answer_in_message" and qids is not None:  # one note per message: its batch
            for qid in qids:
                self.qobs.setdefault(qid, {})["message"] = self.messages
            self.messages += 1
        if qids is None:
            self.obs[key] = v
        else:
            for qid in qids:
                self.qobs.setdefault(qid, {})[key] = v

    def end(self) -> None:
        base = {"path": self.path, "inputs": self.inputs, "observed": self.obs}
        if not self.before:
            self.rec.add(base)
            return
        rows = []
        for slot, (qid, before) in enumerate(self.before.items()):
            after = state(self.db, qid)
            row = {**base, "question": self.qobs.get(qid, {}), "before": before, "after": after}
            if before and after:
                moved = before.pop("_pane") != after.pop("_pane")
                if moved:
                    after["moved_to_another_pane"] = True
            for s in (before, after):
                if s:
                    s.pop("_pane", None)
            rows.append({**row, "slot": slot})
        # the pass: every row of this frame carries the same id, made from the whole frame (so a
        # replay can put a pane's answers back together; identical passes share one id)
        digest = hashlib.sha256(json.dumps(rows, sort_keys=True, ensure_ascii=False).encode()).hexdigest()[:12]
        for row in rows:
            self.rec.add({**row, "pass": digest})


class Recorder:
    def __init__(self):
        self.rows: dict[str, set[str]] = {}
        self.test = "<outside a test>"
        self.lock = threading.Lock()

    def begin(self, path, store, qs, inputs) -> Frame:
        frame = Frame(self, path, inputs)
        frame.track(store, qs)
        return frame

    def event(self, path: str, fields: dict) -> None:
        action = fields.get("action")
        self.add({"path": path, "inputs": {}, "observed": {k: value_of(k, v) for k, v in fields.items()
                                                           if k != "action"}, "action": action})

    def add(self, row: dict) -> None:
        key = json.dumps(row, sort_keys=True, ensure_ascii=False)
        with self.lock:
            self.rows.setdefault(key, set()).add(self.test)

    def table(self) -> list[dict]:
        out = []
        for key in sorted(self.rows):
            row = json.loads(key)
            row["tests"] = sorted(self.rows[key])
            out.append(row)
        return out


def pytest_addoption(parser):
    parser.addoption("--decision-table", default=None, help="write the delivery decision table here (JSON)")


def pytest_configure(config):
    if config.getoption("--decision-table"):
        config._decision_recorder = Recorder()
        decision_trace.RECORDER = config._decision_recorder


@pytest.hookimpl(tryfirst=True)
def pytest_runtest_logstart(nodeid, location):
    if decision_trace.RECORDER is not None:
        decision_trace.RECORDER.test = nodeid


def pytest_sessionfinish(session, exitstatus):
    rec = getattr(session.config, "_decision_recorder", None)
    if rec is None:
        return
    decision_trace.RECORDER = None
    rows = rec.table()
    with open(session.config.getoption("--decision-table"), "w", encoding="utf-8") as fh:
        json.dump({"rows": rows}, fh, ensure_ascii=False)
