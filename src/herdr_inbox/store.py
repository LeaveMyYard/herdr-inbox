"""SQLite storage shared by every MCP server process and the TUI."""

from __future__ import annotations

import json
import os
from contextlib import contextmanager
import re
import sqlite3
from dataclasses import dataclass, fields
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCHEMA = """
CREATE TABLE IF NOT EXISTS questions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at      TEXT NOT NULL,
    pane_id         TEXT,
    tab_id          TEXT,
    tab_label       TEXT,
    session_id      TEXT,
    cwd             TEXT,
    asked_by        TEXT,
    question        TEXT NOT NULL,
    options         TEXT,
    context         TEXT,
    norm            TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'open',
    answer          TEXT,
    answered_at     TEXT,
    delivered_at    TEXT,
    delivery_error  TEXT,
    acknowledged_at TEXT,
    repeat_count    INTEGER NOT NULL DEFAULT 0,
    delivery_attempts INTEGER NOT NULL DEFAULT 0
);
-- Fields added in 0.2 live in a side table, so the questions table keeps the exact
-- column set 0.1.x builds its Question from (0.1.x does Question(**row) on SELECT *).
CREATE TABLE IF NOT EXISTS question_ext (
    id              INTEGER PRIMARY KEY REFERENCES questions(id),
    header          TEXT,
    multi_select    INTEGER NOT NULL DEFAULT 0,
    answer_json     TEXT,
    group_id        INTEGER,
    draft_json      TEXT,
    drafted_at      TEXT
);
CREATE VIEW IF NOT EXISTS questions_v2 AS
    SELECT q.id, q.created_at, q.pane_id, q.tab_id, q.tab_label, q.session_id, q.cwd, q.asked_by,
           q.question, q.options, q.context, q.norm, q.status, q.answer, q.answered_at, q.delivered_at,
           q.delivery_error, q.acknowledged_at, q.repeat_count, q.delivery_attempts,
           e.header, COALESCE(e.multi_select, 0) AS multi_select, e.answer_json,
           COALESCE(e.group_id, q.id) AS group_id, e.draft_json, e.drafted_at
    FROM questions q LEFT JOIN question_ext e ON e.id = q.id;
-- Every tab-label mark ever applied, so any process can strip all of them.
CREATE TABLE IF NOT EXISTS tab_marks (mark TEXT PRIMARY KEY);
CREATE INDEX IF NOT EXISTS questions_status ON questions(status);
CREATE INDEX IF NOT EXISTS questions_session ON questions(session_id);
"""


# Columns of `questions` that pre-0.1 databases may lack. Only ever columns that 0.1.x's
# own Question dataclass knows; everything newer goes into question_ext.
ADDED_COLUMNS = {
    "delivery_attempts": "INTEGER NOT NULL DEFAULT 0",
}


def normalize_options(options) -> list[dict] | None:
    """Plain strings or {label, description} objects -> [{"label", "description"}]."""
    if not options:
        return None
    out = []
    for o in options:
        if isinstance(o, str):
            out.append({"label": o, "description": None})
        else:
            out.append({"label": str(o["label"]), "description": o.get("description") or None})
    return out


def db_path() -> Path:
    if env := os.environ.get("HERDR_INBOX_DB"):
        return Path(env)
    state = os.environ.get("XDG_STATE_HOME") or os.path.expanduser("~/.local/state")
    return Path(state) / "herdr-inbox" / "inbox.db"


def now(delta: timedelta = timedelta()) -> str:
    return (datetime.now(timezone.utc) + delta).isoformat(timespec="seconds")


# An answered question is handed back to a repeat only this long; later the
# same words are probably a new instance ("Proceed with the next step?").
ANSWER_REUSE = timedelta(hours=1)


def normalize(text: str) -> str:
    """lowercase, strip punctuation, collapse whitespace."""
    text = re.sub(r"[^\w\s]", " ", text.lower())
    return " ".join(text.split())


@dataclass
class Question:
    id: int
    created_at: str
    pane_id: str | None
    tab_id: str | None
    tab_label: str | None
    session_id: str | None
    cwd: str | None
    asked_by: str | None
    question: str
    options: list[str] | None
    context: str | None
    norm: str
    status: str
    answer: str | None
    answered_at: str | None
    delivered_at: str | None
    delivery_error: str | None
    acknowledged_at: str | None
    repeat_count: int
    delivery_attempts: int = 0
    header: str | None = None
    multi_select: bool = False
    answer_json: dict | None = None  # {"choices": [...], "notes": {label: note}, "text": str|None}
    group_id: int | None = None  # questions filed by one ask_operator call share it
    draft_json: dict | None = None  # operator's unsent answer, same shape as answer_json
    drafted_at: str | None = None

    @property
    def group(self) -> int:
        return self.group_id or self.id

    @property
    def ref(self) -> str:
        return f"Q{self.id}"

    @classmethod
    def from_row(cls, row: sqlite3.Row) -> "Question":
        known = {f.name for f in fields(cls)}
        data = {k: v for k, v in dict(row).items() if k in known}  # newer columns are ignored
        data["options"] = normalize_options(json.loads(data["options"])) if data["options"] else None
        data["multi_select"] = bool(data.get("multi_select"))
        for key in ("answer_json", "draft_json"):
            data[key] = json.loads(data[key]) if data.get(key) else None
        return cls(**data)

    def brief(self) -> dict:
        """What an agent gets back about its own question."""
        out = {"id": self.id, "status": self.status, "question": self.question}
        if self.status == "answered":
            out["answer"] = self.answer
            if self.answer_json:
                out["answer_detail"] = self.answer_json
        return out


class Store:
    def __init__(self, path: Path | str | None = None):
        self.path = Path(path) if path else db_path()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(self.path, timeout=15, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA busy_timeout=15000")
        self.db.executescript(SCHEMA)
        columns = {r[1] for r in self.db.execute("PRAGMA table_info(questions)")}
        for name, decl in ADDED_COLUMNS.items():  # databases from before the column existed
            if name not in columns:
                self.db.execute(f"ALTER TABLE questions ADD COLUMN {name} {decl}")

    def close(self) -> None:
        self.db.close()

    @contextmanager
    def transaction(self):
        """Write lock for read-then-write sequences (other processes wait on busy_timeout)."""
        self.db.execute("BEGIN IMMEDIATE")
        try:
            yield
        except BaseException:
            self.db.execute("ROLLBACK")
            raise
        self.db.execute("COMMIT")

    def __enter__(self) -> "Store":
        return self

    def __exit__(self, *exc) -> None:
        self.close()

    # -- reads ---------------------------------------------------------------

    def get(self, qid: int) -> Question | None:
        row = self.db.execute("SELECT * FROM questions_v2 WHERE id = ?", (qid,)).fetchone()
        return Question.from_row(row) if row else None

    def _owner_clause(self, session_id: str | None, pane_id: str | None) -> tuple[str, tuple]:
        """Questions belong to a session; without one, to the pane."""
        if session_id:
            return "session_id = ?", (session_id,)
        if pane_id:
            return "session_id IS NULL AND pane_id = ?", (pane_id,)
        return "0", ()

    def find_duplicate(self, norm: str, session_id: str | None, pane_id: str | None) -> Question | None:
        where, args = self._owner_clause(session_id, pane_id)
        row = self.db.execute(
            f"SELECT * FROM questions_v2 WHERE {where} AND norm = ?"
            " AND (status = 'open' OR (status = 'answered' AND answered_at >= ?))"
            " ORDER BY status = 'open' DESC, id DESC LIMIT 1",
            (*args, norm, now(-ANSWER_REUSE)),
        ).fetchone()
        return Question.from_row(row) if row else None

    def mine(self, session_id: str | None, pane_id: str | None, include_closed: bool = False) -> list[Question]:
        where, args = self._owner_clause(session_id, pane_id)
        if not include_closed:
            where += " AND status IN ('open', 'answered')"
        rows = self.db.execute(f"SELECT * FROM questions_v2 WHERE {where} ORDER BY id", args).fetchall()
        return [Question.from_row(r) for r in rows]

    def list(self, include_closed: bool = False) -> list[Question]:
        where = "1" if include_closed else "status = 'open'"
        rows = self.db.execute(f"SELECT * FROM questions_v2 WHERE {where} ORDER BY id DESC").fetchall()
        return [Question.from_row(r) for r in rows]

    def open_counts(self) -> dict[str, int]:
        rows = self.db.execute(
            "SELECT pane_id, COUNT(*) FROM questions WHERE status = 'open' AND pane_id IS NOT NULL GROUP BY pane_id"
        ).fetchall()
        return {r[0]: r[1] for r in rows}

    def known_panes(self) -> list[str]:
        rows = self.db.execute("SELECT DISTINCT pane_id FROM questions WHERE pane_id IS NOT NULL").fetchall()
        return [r[0] for r in rows]

    # -- writes --------------------------------------------------------------

    def insert(
        self,
        question: str,
        *,
        options: list[str] | None = None,
        context: str | None = None,
        asked_by: str | None = None,
        pane_id: str | None = None,
        tab_id: str | None = None,
        tab_label: str | None = None,
        session_id: str | None = None,
        cwd: str | None = None,
        header: str | None = None,
        multi_select: bool = False,
        group_id: int | None = None,
    ) -> Question:
        options = normalize_options(options)
        cur = self.db.execute(
            "INSERT INTO questions (created_at, pane_id, tab_id, tab_label, session_id, cwd, asked_by,"
            " question, options, context, norm) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                now(), pane_id, tab_id, tab_label, session_id, cwd, asked_by, question,
                json.dumps(options) if options else None, context, normalize(question),
            ),
        )
        self.db.execute(
            "INSERT INTO question_ext (id, header, multi_select, group_id) VALUES (?, ?, ?, ?)",
            (cur.lastrowid, header, int(multi_select), group_id or cur.lastrowid),  # first names the group
        )
        return self.get(cur.lastrowid)

    def group(self, group_id: int, include_closed: bool = False) -> list[Question]:
        where = "" if include_closed else " AND status = 'open'"
        rows = self.db.execute(
            f"SELECT * FROM questions_v2 WHERE group_id = ?{where} ORDER BY id", (group_id,)
        ).fetchall()
        return [Question.from_row(r) for r in rows]

    def set_draft(self, qid: int, detail: dict | None) -> bool:
        """Store (or with None clear) the operator's unsent answer to an open question."""
        cur = self.db.execute(
            "INSERT INTO question_ext (id, group_id, draft_json, drafted_at)"
            " SELECT id, id, ?, ? FROM questions WHERE id = ? AND status = 'open'"
            " ON CONFLICT(id) DO UPDATE SET draft_json = excluded.draft_json, drafted_at = excluded.drafted_at",
            (json.dumps(detail) if detail else None, now() if detail else None, qid),
        )
        return cur.rowcount == 1

    def drafted(self) -> list[Question]:
        rows = self.db.execute(
            "SELECT * FROM questions_v2 WHERE status = 'open' AND draft_json IS NOT NULL ORDER BY id"
        ).fetchall()
        return [Question.from_row(r) for r in rows]

    def bump_repeat(self, qid: int) -> None:
        self.db.execute("UPDATE questions SET repeat_count = repeat_count + 1 WHERE id = ?", (qid,))

    def set_answer(self, qid: int, answer: str, detail: dict | None = None) -> bool:
        with self.transaction():
            return self._set_answer(qid, answer, detail)

    def _set_answer(self, qid: int, answer: str, detail: dict | None) -> bool:
        cur = self.db.execute(
            "UPDATE questions SET status = 'answered', answer = ?, answered_at = ?,"
            " delivered_at = NULL, delivery_error = NULL, acknowledged_at = NULL WHERE id = ? AND status = 'open'",
            (answer, now(), qid),
        )
        if cur.rowcount != 1:
            return False
        self.db.execute(
            "INSERT INTO question_ext (id, group_id, answer_json) VALUES (?, ?, ?)"
            " ON CONFLICT(id) DO UPDATE SET answer_json = excluded.answer_json, draft_json = NULL, drafted_at = NULL",
            (qid, qid, json.dumps(detail) if detail else None),
        )
        return True

    def set_status(self, qid: int, status: str, note: str | None = None) -> bool:
        """Close an open question (dismissed / withdrawn); ``note`` goes into ``answer``."""
        cur = self.db.execute(
            "UPDATE questions SET status = ?, answer = ?, answered_at = ? WHERE id = ? AND status = 'open'",
            (status, note, now(), qid),
        )
        return cur.rowcount == 1

    def record_delivery(self, qid: int, error: str | None) -> None:
        if error:
            self.db.execute(
                "UPDATE questions SET delivery_error = ?, delivery_attempts = delivery_attempts + 1 WHERE id = ?",
                (error, qid),
            )
        else:  # typed into the asking conversation: that counts as seen
            self.db.execute(
                "UPDATE questions SET delivered_at = ?, acknowledged_at = COALESCE(acknowledged_at, ?),"
                " delivery_error = NULL WHERE id = ?",
                (now(), now(), qid),
            )

    def undelivered(self, pane_id: str, stale_errors: tuple[str, ...], max_stale: int) -> list[Question]:
        """Answers for ``pane_id`` whose delivery failed and that the agent has not seen.

        Errors starting with one of ``stale_errors`` (the conversation is gone) are
        given up after ``max_stale`` attempts; transient ones are retried indefinitely.
        """
        stale = " OR ".join("delivery_error LIKE ?" for _ in stale_errors) or "0"
        rows = self.db.execute(
            "SELECT * FROM questions_v2 WHERE pane_id = ? AND status = 'answered' AND delivered_at IS NULL"
            f" AND acknowledged_at IS NULL AND delivery_error IS NOT NULL"
            f" AND NOT (({stale}) AND delivery_attempts >= ?) ORDER BY id",
            (pane_id, *(e + "%" for e in stale_errors), max_stale),
        ).fetchall()
        return [Question.from_row(r) for r in rows]

    def acknowledge(self, qids: list[int]) -> None:
        self.db.executemany(
            "UPDATE questions SET acknowledged_at = ? WHERE id = ? AND acknowledged_at IS NULL",
            [(now(), q) for q in qids],
        )

    def unacknowledged_answers(self, session_id: str | None, pane_id: str | None) -> list[Question]:
        where, args = self._owner_clause(session_id, pane_id)
        rows = self.db.execute(
            f"SELECT * FROM questions_v2 WHERE {where} AND status = 'answered' AND acknowledged_at IS NULL ORDER BY id",
            args,
        ).fetchall()
        return [Question.from_row(r) for r in rows]
