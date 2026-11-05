"""The remote side's part of the CLI golden (``tools/capture_cli.py``): ``inbox export`` and
``inbox apply`` over a database of their own (``fresh``), with requests on stdin.

Every question and alert here is created at ``CREATED``, a fixed time, so its key (``qkey``,
``akey``) is the same at every capture; the export windows (answers of the last 7 days, alerts
closed in the last hour) are relative to now. In their stdout, times other than ``CREATED`` are
``<time>``, the version ``<version>``, a ``db_id`` other than ``DB`` (the one the first export
creates) ``<db_id>``, and a JSON parser's reason after ``bad request: `` ``<reason>`` (Python's
``json`` words them, not the Rust parser). The tables after them are ``REMOTE_TABLES`` and the
database's id (``db_id``).
"""

from __future__ import annotations

import hashlib
import json
import re

DB = "d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0"
CREATED = "2001-01-01T00:00:00+00:00"
S_A = "aaaaaaaa-0000-4000-8000-00000000000a"
REMOTE_TABLES = ("questions", "question_ext", "question_groups", "alerts", "applied_requests", "changes")
TABS = [["w1:p1", "w1:t1"], ["w1:p2", "w1:t2"]]

_TIME = re.compile(r'"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+00:00"')
_VERSION = re.compile(r'^\{"version": "[^"]*"')
_DB_ID = re.compile(r'"db_id": "([0-9a-f]{32})"')
_REASON = re.compile(r'"bad request: (?!not an object")[^\n]*"\}$', re.M)


def mask(stdout: str) -> str:
    """An export's or apply's stdout as the golden keeps it (see the module docstring)."""
    out = _TIME.sub(lambda m: m.group(0) if m.group(0) == f'"{CREATED}"' else '"<time>"', stdout)
    out = _VERSION.sub('{"version": "<version>"', out)
    out = _DB_ID.sub(lambda m: m.group(0) if m.group(1) == DB else '"db_id": "<db_id>"', out)
    return _REASON.sub('"bad request: <reason>"}', out)


def qkey(created: str, norm: str, session: str | None) -> str:
    """Python ``remote_api.qkey``'s formula, so a request can name a question."""
    return hashlib.sha256(f"{DB}|{created}|{norm}|{session or ''}".encode()).hexdigest()[:32]


def akey(aid: int, title: str) -> str:
    return hashlib.sha256(f"{DB}|alert|{aid}|{CREATED}|{title}".encode()).hexdigest()[:32]


def _ago(spec: str | None) -> str:
    return "NULL" if spec is None else f"strftime('%Y-%m-%dT%H:%M:%S+00:00', 'now', '{spec}')"


QUESTIONS: dict[int, tuple] = {}  # id -> (question, session): what a request's qkey is made of


def _q(qid, question, status="open", pane=None, session=None, answer=None, answered=None, delivered=None,
       acked=None, options=None):
    QUESTIONS[qid] = (question, session)
    return ("sql", {"sql": "INSERT INTO questions (id, created_at, pane_id, tab_id, tab_label, session_id, cwd,"
                           " asked_by, question, options, norm, status, answer, answered_at, delivered_at,"
                           " acknowledged_at, delivery_attempts)"
                           f" VALUES (?, ?, ?, ?, 'tab', ?, '/r', 'lead', ?, ?, ?, ?, ?, {_ago(answered)},"
                           f" {_ago(delivered)}, {_ago(acked)}, 2)",
                    "params": [qid, CREATED, pane, "w1:t1" if pane else None, session, question, options,
                               question.lower(), status, answer]})


def _ext(qid, **cols):
    names = ", ".join(cols)
    marks = ", ".join("?" * len(cols))
    return ("sql", {"sql": f"INSERT INTO question_ext (id, {names}) VALUES (?, {marks})",
                    "params": [qid, *cols.values()]})


def _alert(aid, title, status="open", kind="agent", message=None, acked=None, closed=None, pane="w1:p1"):
    return ("sql", {"sql": "INSERT INTO alerts (id, created_at, kind, severity, title, message, pane_id, tab_id,"
                           " tab_label, session_id, asked_by, herdr_server, status, acked_at, closed_at)"
                           f" VALUES (?, ?, ?, 'urgent', ?, ?, ?, 'w1:t1', 'tab', ?, 'lead', '/srv',"
                           f" ?, {_ago(acked)}, {_ago(closed)})",
                    "params": [aid, CREATED, kind, title, message, pane, S_A, status]})


def _sql(sql):
    return ("sql", {"sql": sql, "params": []})


def export(request, **kw):
    return ("cli", {"argv": ["export"], "stdin": request if isinstance(request, str) else json.dumps(request), **kw})


def apply(rid, action, qid, detail=None, *, db=DB, key=None, **kw):
    """An apply request for question (or alert) ``qid``, its key made from its seed unless given."""
    if key is None and isinstance(qid, int) and qid in QUESTIONS:
        question, session = QUESTIONS[qid]
        key = qkey(CREATED, question.lower(), session)
    request = {"request_id": rid, "db_id": db, "id": qid, "qkey": key, "action": action}
    if detail is not None:
        request["detail"] = detail
    return ("cli", {"argv": ["apply"], "stdin": json.dumps(request), **kw})


def raw_apply(request, **kw):
    return ("cli", {"argv": ["apply"], "stdin": request if isinstance(request, str) else json.dumps(request), **kw})


OPTIONS = json.dumps([{"label": "yes", "description": "ship it", "preview": None},
                      {"label": "no", "description": None, "preview": "```\nkeep\n```"}])

# no database yet: export and apply never create or migrate it (the worker, started, does)
BEFORE = [
    export({}),
    raw_apply({"request_id": "r0", "action": "status"}),
    export("[1]"),
    raw_apply("{not json"),
]

REMOTE_OPS = [
    ("fresh", {}),
    ("cli", {"argv": ["export", "-h"]}),
    ("cli", {"argv": ["apply", "extra"]}),
    export({"cursor": None}),  # an empty database: its id is created now
    _sql(f"UPDATE meta SET value = '{DB}' WHERE key = 'db_id'"),
    _q(1, "Deploy now? ünïcode 🚀", pane="w1:p1", session=S_A),
    _ext(1, group_id=1, header="Deploy", multi_select=0, options_json=OPTIONS),
    ("sql", {"sql": "INSERT INTO question_groups (group_id, title) VALUES (1, 'Release')", "params": []}),
    _q(2, "Legacy options?", options='["a", "b"]'),
    _q(3, "Answered lately?", status="answered", pane="w1:p2", answer="yes", answered="-1 days"),
    _q(4, "Answered long ago?", status="answered", answer="old", answered="-8 days"),
    _q(5, "Answered and seen?", status="answered", answer="seen", answered="-1 hours", acked="-1 hours"),
    _q(6, "Dismissed?", status="dismissed", answered="-1 hours"),
    _q(7, "Hidden, its agent gone, several picks?", pane="w1:p2", session=S_A),
    _ext(7, group_id=1, multi_select=1, hidden_at=CREATED, agent_gone_at=CREATED,
         options_json=json.dumps(["p", "q", "r"])),
    _q(8, "Delivered?", status="answered", pane="w1:p1", answer="sent", answered="-2 hours", delivered="-2 hours"),
    _q(9, "Not delivered yet?", status="answered", pane="w1:p1", answer="later", answered="-2 hours"),
    _q(10, "Withdrawn?", status="withdrawn", answered="-1 hours"),
    _q(11, "Picks as text?", pane="w1:p1"),
    _q(12, 'Quote "this", (and) that·', pane="w1:p2", session=S_A),
    _alert(1, "Check the build"),
    _alert(2, "Looked at already", status="acked", acked="-10 minutes", message="seen"),
    _alert(3, "Closed long ago", status="withdrawn", closed="-2 hours"),
    _alert(4, "Agent blocked", kind="blocked", pane=None),
    # the change feed starts over at 101
    _sql("DELETE FROM changes"),
    _sql("UPDATE sqlite_sequence SET seq = 100 WHERE name = 'changes'"),
    export({"cursor": None}),
    export({"cursor": {"db_id": DB, "seq": 100}}),  # nothing changed
    _sql("UPDATE questions SET repeat_count = 1 WHERE id = 2"),
    _sql("DELETE FROM questions WHERE id = 6"),
    export({"cursor": {"db_id": DB, "seq": 100}}),
    export({"cursor": {"db_id": DB, "seq": 101}, "full": 0}),
    export({"cursor": {"db_id": "e" * 32, "seq": 101}}),  # another database: full
    export({"cursor": {"db_id": DB, "seq": 50}}),  # pruned past: full
    export({"cursor": {"db_id": DB, "seq": True}}),  # an int to Python, and pruned past
    export({"cursor": {"db_id": DB, "seq": "101"}}),
    export({"cursor": {"db_id": DB, "seq": 101.0}}),
    export({"cursor": {"db_id": DB, "seq": 101}, "full": "yes"}),
    export({"cursor": [DB, 101]}),
    export({"cursor": {"db_id": DB, "seq": 100}, "extra": [1, {"x": None}]}),  # oldest - 1: still incremental
    export({"cursor": {"db_id": DB, "seq": 99}}),  # one less: full
    export("[]"),
    export("null"),
    export("{bad"),
    export(""),
    export('{"cursor": null} trailing'),
    # answers
    apply("r1", "answer", 1, {"choices": ["yes"], "notes": {"yes": " now ", "no": "dropped"}, "text": "  and tag it "},
          config='tab_prefix = "❓"\n', pane_tabs=TABS, labels={"w1:t1": "❓4 tab"}),
    apply("r1", "answer", 1, {"choices": ["yes"]}),  # the lost-reply retry: from the record
    apply("r2", "answer", 1, {"choices": ["yes"], "notes": {"yes": "now"}, "text": "and tag it"}),  # ours, unrecorded
    apply("r3", "answer", 1, {"choices": ["no"]}),
    raw_apply({"request_id": "r1", "action": "status"}),
    raw_apply({"request_id": "late", "action": "status"}),  # never sent: a tombstone
    apply("late", "answer", 2, {"text": "go"}),
    raw_apply({"request_id": "late", "action": "status"}),
    apply("r4", "answer", 2, {"text": "go"}, db="0" * 32),
    apply("r5", "answer", 2, {"text": "go"}, key="f" * 32),
    apply("r6", "answer", 999, {"text": "go"}),
    apply("r7", "answer", "2", {"text": "go"}, key=qkey(CREATED, "legacy options?", None)),
    apply("r8", "dismiss", True, key=qkey(CREATED, "deploy now? ünïcode 🚀", S_A)),  # id true is Q1
    apply("r9", "answer", 2, {"text": "   "}),
    apply("r10", "answer", 2, None),
    apply("r11", "answer", 2, {"choices": [], "notes": {"x": "y"}, "text": None}),
    apply("r12", "answer", 11, {"choices": "ab", "notes": {"b": "  bee "}}),
    apply("r13", "answer", 12, {"choices": {"a, b": 1, 'say "hi"': 2}, "notes": []}),
    apply("r14", "answer", 4, {"text": "late"}),
    apply("r15", "answer", 10, {"text": "late"}),
    apply("r16", "answer", 2, {"text": " é ", "choices": None}, pane_tabs=TABS),
    # dismiss, hide, deliver
    apply("d1", "dismiss", 7, pane_tabs=TABS, labels={"w1:t2": "❓2 two"}),
    apply("d2", "dismiss", 7),
    apply("h1", "hide", 8),
    apply("h2", "hide", 2),
    apply("v1", "deliver", 8),
    apply("v2", "deliver", 9),
    apply("v3", "deliver", 11),
    apply("v4", "deliver", 1),
    # alerts
    apply("a1", "ack_alert", 1, key=akey(1, "Check the build"), pane_tabs=TABS),
    apply("a2", "ack_alert", 1, key=akey(1, "Check the build")),
    apply("a3", "ack_alert", 4, key=akey(1, "Check the build")),
    apply("a4", "ack_alert", 99, key=akey(99, "x")),
    apply("a5", "ack_alert", 4, key=akey(4, "Agent blocked"), db="1" * 32),
    apply("a6", "ack_alert", 4, key=akey(4, "Agent blocked")),
    apply("a7", "ack_alert", 3, key=akey(3, "Closed long ago")),
    # malformed requests: refused, unrecorded
    raw_apply({"request_id": "", "action": "status"}),
    raw_apply({"request_id": "x" * 65, "action": "answer"}),
    raw_apply({"request_id": "é" * 64, "action": "status"}),  # 64 characters
    raw_apply({"request_id": 5, "action": "answer"}),
    raw_apply({"request_id": "b1", "action": "bogus"}),
    raw_apply({"request_id": "b2", "action": ["answer"]}),
    raw_apply({"request_id": "b3"}),
    raw_apply({"request_id": "b4", "action": "answer", "id": 2}),  # no db_id: reset
    raw_apply("[1, 2]"),
    raw_apply(""),
    # what the puller sees after all that
    export({"cursor": {"db_id": DB, "seq": 102}}),
    export({"cursor": None}),
]
