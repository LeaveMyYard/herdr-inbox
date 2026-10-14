# Verbatim copy of src/herdr_inbox/store.py as released in 0.1.x (branch poc), used to prove that
# processes still running 0.1.x keep working on a database that 0.2 has opened and written.

"""SQLite storage shared by every MCP server process and the TUI."""

from __future__ import annotations

import json
import os
from contextlib import contextmanager
import re
import sqlite3
from dataclasses import dataclass
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
CREATE INDEX IF NOT EXISTS questions_status ON questions(status);
CREATE INDEX IF NOT EXISTS questions_session ON questions(session_id);
"""


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

    @property
    def ref(self) -> str:
        return f"Q{self.id}"

    @classmethod
    def from_row(cls, row: sqlite3.Row) -> "Question":
        data = dict(row)
        data["options"] = json.loads(data["options"]) if data["options"] else None
        return cls(**data)

    def brief(self) -> dict:
        """What an agent gets back about its own question."""
        out = {"id": self.id, "status": self.status, "question": self.question}
        if self.status == "answered":
            out["answer"] = self.answer
        return out


class Store:
    def __init__(self, path: Path | str | None = None):
        self.path = Path(path) if path else db_path()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(self.path, timeout=5, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA busy_timeout=5000")
        self.db.executescript(SCHEMA)
        columns = {r[1] for r in self.db.execute("PRAGMA table_info(questions)")}
        if "delivery_attempts" not in columns:  # databases from before the column existed
            self.db.execute("ALTER TABLE questions ADD COLUMN delivery_attempts INTEGER NOT NULL DEFAULT 0")

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
        row = self.db.execute("SELECT * FROM questions WHERE id = ?", (qid,)).fetchone()
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
            f"SELECT * FROM questions WHERE {where} AND norm = ?"
            " AND (status = 'open' OR (status = 'answered' AND answered_at >= ?))"
            " ORDER BY status = 'open' DESC, id DESC LIMIT 1",
            (*args, norm, now(-ANSWER_REUSE)),
        ).fetchone()
        return Question.from_row(row) if row else None

    def mine(self, session_id: str | None, pane_id: str | None, include_closed: bool = False) -> list[Question]:
        where, args = self._owner_clause(session_id, pane_id)
        if not include_closed:
            where += " AND status IN ('open', 'answered')"
        rows = self.db.execute(f"SELECT * FROM questions WHERE {where} ORDER BY id", args).fetchall()
        return [Question.from_row(r) for r in rows]

    def list(self, include_closed: bool = False) -> list[Question]:
        where = "1" if include_closed else "status = 'open'"
        rows = self.db.execute(f"SELECT * FROM questions WHERE {where} ORDER BY id DESC").fetchall()
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
    ) -> Question:
        cur = self.db.execute(
            "INSERT INTO questions (created_at, pane_id, tab_id, tab_label, session_id, cwd, asked_by,"
            " question, options, context, norm) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                now(), pane_id, tab_id, tab_label, session_id, cwd, asked_by, question,
                json.dumps(options) if options else None, context, normalize(question),
            ),
        )
        return self.get(cur.lastrowid)

    def bump_repeat(self, qid: int) -> None:
        self.db.execute("UPDATE questions SET repeat_count = repeat_count + 1 WHERE id = ?", (qid,))

    def set_answer(self, qid: int, answer: str) -> bool:
        cur = self.db.execute(
            "UPDATE questions SET status = 'answered', answer = ?, answered_at = ?,"
            " delivered_at = NULL, delivery_error = NULL, acknowledged_at = NULL WHERE id = ? AND status = 'open'",
            (answer, now(), qid),
        )
        return cur.rowcount == 1

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
            "SELECT * FROM questions WHERE pane_id = ? AND status = 'answered' AND delivered_at IS NULL"
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
            f"SELECT * FROM questions WHERE {where} AND status = 'answered' AND acknowledged_at IS NULL ORDER BY id",
            args,
        ).fetchall()
        return [Question.from_row(r) for r in rows]
