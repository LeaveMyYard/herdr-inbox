"""SQLite storage shared by every MCP server process and the TUI."""

from __future__ import annotations

import json
import os
import uuid
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
    drafted_at      TEXT,
    options_json    TEXT,
    hidden_at       TEXT,
    verified_at     TEXT,
    reset_count     INTEGER,
    pending_since   TEXT
);
-- Transcript path per Claude session, recorded by the SessionStart hook (0.5.1): delivery
-- is verified by finding the typed answer in it.
CREATE TABLE IF NOT EXISTS sessions (
    session_id      TEXT PRIMARY KEY,
    transcript_path TEXT
);
-- One-off migrations and other flags.
CREATE TABLE IF NOT EXISTS meta (
    key             TEXT PRIMARY KEY,
    value           TEXT
);
-- One row per group (one ask_operator call) that was given a title (0.5).
CREATE TABLE IF NOT EXISTS question_groups (
    group_id        INTEGER PRIMARY KEY,
    title           TEXT
);
-- Tabs whose label the inbox decorated, and with which marks: only these are ever
-- stripped, so an operator label such as "Q4 roadmap" is never touched.
CREATE TABLE IF NOT EXISTS tab_decor (
    tab_id          TEXT NOT NULL,
    mark            TEXT NOT NULL,
    PRIMARY KEY (tab_id, mark)
);
CREATE INDEX IF NOT EXISTS questions_status ON questions(status);
CREATE INDEX IF NOT EXISTS questions_session ON questions(session_id);
"""


# Columns of `questions` that pre-0.1 databases may lack. Only ever columns that 0.1.x's
# own Question dataclass knows; everything newer goes into question_ext.
ADDED_COLUMNS = {
    "delivery_attempts": "INTEGER NOT NULL DEFAULT 0",
}


# Read view, versioned: a release that changes it bumps the name (CREATE VIEW IF NOT EXISTS
# would keep an old definition). Older views are NEVER dropped: processes of the previous
# release keep running until their sessions restart, and each queries its own view.
VIEW = "questions_v6"
OLD_VIEWS: tuple[str, ...] = ()
VIEW_SQL = f"""
CREATE VIEW IF NOT EXISTS {VIEW} AS
    SELECT q.id, q.created_at, q.pane_id, q.tab_id, q.tab_label, q.session_id, q.cwd, q.asked_by,
           q.question, q.options, q.context, q.norm, q.status, q.answer, q.answered_at, q.delivered_at,
           q.delivery_error, q.acknowledged_at, q.repeat_count, q.delivery_attempts,
           e.header, COALESCE(e.multi_select, 0) AS multi_select, e.answer_json,
           COALESCE(e.group_id, q.id) AS group_id, e.draft_json, e.drafted_at, e.options_json, e.hidden_at,
           g.title AS group_title, e.verified_at
    FROM questions q LEFT JOIN question_ext e ON e.id = q.id
    LEFT JOIN question_groups g ON g.group_id = COALESCE(e.group_id, q.id);
"""
EXT_ADDED_COLUMNS = {"options_json": "TEXT", "hidden_at": "TEXT", "verified_at": "TEXT", "reset_count": "INTEGER", "pending_since": "TEXT"}  # question_ext is ours alone; 0.1.x never reads it


MAX_TITLE = 60


def clean_title(title: str | None) -> str | None:
    """One line, no control characters, at most MAX_TITLE characters."""
    if not title:
        return None
    text = " ".join("".join(ch if ch.isprintable() else " " for ch in str(title)).split())
    if len(text) > MAX_TITLE:
        text = text[: MAX_TITLE - 1] + "…"
    return text or None


def normalize_options(options) -> list[dict] | None:
    """Plain strings or {label, description, preview} objects -> [{"label", "description", "preview"}]."""
    if not options:
        return None
    out = []
    for o in options:
        if isinstance(o, str):
            out.append({"label": o, "description": None, "preview": None})
        else:
            out.append({"label": str(o["label"]), "description": o.get("description") or None,
                        "preview": o.get("preview") or None})
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
# An answered row with no delivery attempt this long after answering was orphaned
# (the answering process died between answer and delivery): the retry picks it up.
NEVER_ATTEMPTED_GRACE = timedelta(seconds=60)
# delivery_error prefix for an answer waiting for its agent to be idle (not a failure).
PENDING = "pending: "
RESET_KEY = "0.5.1 unverified acknowledgements reset"
RESET_WINDOW = timedelta(days=7)
SIGNATURE_WINDOW = timedelta(hours=24)
RECONCILE_BATCH = 200  # rows per open; the 7-day sweep is done only once a pass sees fewer
MAX_RESETS = 2
# A delivery claim older than this belongs to a process that died mid-send.
CLAIM_STALE = timedelta(seconds=60)


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
    group_title: str | None = None  # the ask_operator call's title (0.5), if the agent gave one
    verified_at: str | None = None  # the typed answer was seen in the session transcript (0.5.1)

    @property
    def group(self) -> int:
        return self.group_id or self.id

    @property
    def ref(self) -> str:
        return f"Q{self.id}"

    @classmethod
    def from_row(cls, row: sqlite3.Row) -> "Question":
        raw = dict(row)
        known = {f.name for f in fields(cls)}
        data = {k: v for k, v in raw.items() if k in known}  # newer columns are ignored
        # full option objects live in question_ext; questions.options holds labels only (0.1.x reads it)
        opts = raw.get("options_json") or data.get("options")
        data["options"] = normalize_options(json.loads(opts)) if opts else None
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
    def __init__(self, path: Path | str | None = None, timeout: float = 15.0):
        self.path = Path(path) if path else db_path()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(self.path, timeout=timeout, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute(f"PRAGMA busy_timeout={int(timeout * 1000)}")
        self.db.executescript(SCHEMA)
        ext_columns = {r[1] for r in self.db.execute("PRAGMA table_info(question_ext)")}
        for name, decl in EXT_ADDED_COLUMNS.items():
            if name not in ext_columns:
                self.db.execute(f"ALTER TABLE question_ext ADD COLUMN {name} {decl}")
        for old in OLD_VIEWS:
            self.db.execute(f"DROP VIEW IF EXISTS {old}")
        self.db.executescript(VIEW_SQL)
        self._reconcile_unverified()
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
        row = self.db.execute(f"SELECT * FROM {VIEW} WHERE id = ?", (qid,)).fetchone()
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
            f"SELECT * FROM {VIEW} WHERE {where} AND norm = ?"
            " AND (status = 'open' OR (status = 'answered' AND answered_at >= ?))"
            " ORDER BY status = 'open' DESC, id DESC LIMIT 1",
            (*args, norm, now(-ANSWER_REUSE)),
        ).fetchone()
        return Question.from_row(row) if row else None

    def mine(self, session_id: str | None, pane_id: str | None, include_closed: bool = False) -> list[Question]:
        where, args = self._owner_clause(session_id, pane_id)
        if not include_closed:
            where += " AND status IN ('open', 'answered')"
        rows = self.db.execute(f"SELECT * FROM {VIEW} WHERE {where} ORDER BY id", args).fetchall()
        return [Question.from_row(r) for r in rows]

    def list(self, include_closed: bool = False) -> list[Question]:
        where = "1" if include_closed else "status = 'open'"
        rows = self.db.execute(f"SELECT * FROM {VIEW} WHERE {where} ORDER BY id DESC").fetchall()
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
                json.dumps([o["label"] for o in options]) if options else None, context, normalize(question),
            ),
        )
        self.db.execute(
            "INSERT INTO question_ext (id, header, multi_select, group_id, options_json) VALUES (?, ?, ?, ?, ?)",
            (cur.lastrowid, header, int(multi_select), group_id or cur.lastrowid,  # first names the group
             json.dumps(options) if options else None),
        )
        return self.get(cur.lastrowid)

    def set_group_title(self, group_id: int, title: str | None) -> None:
        title = clean_title(title)
        if title:
            self.db.execute("INSERT OR REPLACE INTO question_groups (group_id, title) VALUES (?, ?)",
                            (group_id, title))

    def group(self, group_id: int, include_closed: bool = False) -> list[Question]:
        where = "" if include_closed else " AND status = 'open'"
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE group_id = ?{where} ORDER BY id", (group_id,)
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

    def claim_delivery(self, qid: int) -> str | None:
        """Compare-and-set before any delivery outcome is written: only one process handles a
        given answer at a time. Returns the claim token, or None if another process holds it.

        The claim is a ``sending:<time>:<token>`` marker in ``delivery_error``; one older than
        ``CLAIM_STALE`` (its process died mid-send) can be taken over.
        """
        claim = f"sending:{now()}:{uuid.uuid4().hex[:8]}"
        cur = self.db.execute(
            "UPDATE questions SET delivery_error = ? WHERE id = ? AND status = 'answered' AND delivered_at IS NULL"
            " AND acknowledged_at IS NULL"  # the agent already fetched it (check_answers): never retype
            " AND (delivery_error IS NULL OR delivery_error NOT LIKE 'sending:%' OR delivery_error < ?)",
            (claim, qid, f"sending:{now(-CLAIM_STALE)}"),
        )
        return claim if cur.rowcount == 1 else None

    def release_claim(self, qid: int, claim: str, previous: str | None) -> None:
        """Give a claim back untouched (e.g. a retry that decided to wait)."""
        self.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ? AND delivery_error = ?",
                        (previous, qid, claim))

    def failed_deliveries(self) -> list[Question]:
        """Answers whose delivery failed and that the agent has not fetched, unless hidden."""
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE status = 'answered' AND delivered_at IS NULL"
            # a claim in flight is not a failure; one stranded by a dead sender is, and so is
            # an answer whose process died before it was ever claimed
            " AND ((delivery_error IS NOT NULL AND (delivery_error NOT LIKE 'sending:%' OR delivery_error < ?))"
            "      OR (delivery_error IS NULL AND answered_at < ?))"
            " AND acknowledged_at IS NULL AND hidden_at IS NULL ORDER BY id",
            (f"sending:{now(-CLAIM_STALE)}", now(-NEVER_ATTEMPTED_GRACE)),
        ).fetchall()
        return [Question.from_row(r) for r in rows]

    def hide_failed(self, qid: int) -> None:
        """Stop showing a failed delivery in the popup (the agent can still check_answers it)."""
        self.db.execute(
            "INSERT INTO question_ext (id, group_id, hidden_at) VALUES (?, ?, ?)"
            " ON CONFLICT(id) DO UPDATE SET hidden_at = excluded.hidden_at",
            (qid, qid, now()),
        )

    def drafted(self) -> list[Question]:
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE status = 'open' AND draft_json IS NOT NULL ORDER BY id"
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

    def record_delivery(self, qid: int, error: str | None, acknowledge: bool = True,
                        claim: str | None = None, verified: bool = False) -> None:
        """Write a delivery outcome; with ``claim``, only while that claim is still ours.

        A sent prompt is NOT an answer seen: only a ``verified`` delivery (found in the
        session transcript) sets ``acknowledged_at``. An unverified one leaves the answer for
        check_answers.
        """
        mine, args = (" AND delivery_error = ?", (claim,)) if claim else ("", ())
        if error:
            self.db.execute(
                "UPDATE questions SET delivery_error = ?, delivery_attempts = delivery_attempts + 1"
                f" WHERE id = ?{mine}",
                (error, qid, *args),
            )
            return
        # one transaction: another process's reconcile pass must never see the row delivered
        # and acknowledged together (the pre-0.5.1 signature) without its verified_at
        with self.transaction():
            cur = self.db.execute(
                "UPDATE questions SET delivered_at = ?, acknowledged_at = COALESCE(acknowledged_at, ?),"
                f" delivery_error = NULL WHERE id = ?{mine}",
                (now(), now() if acknowledge and verified else None, qid, *args),
            )
            if cur.rowcount:
                self.db.execute("UPDATE question_ext SET pending_since = NULL WHERE id = ?", (qid,))
            if verified and cur.rowcount:
                self.db.execute(
                    "INSERT INTO question_ext (id, group_id, verified_at) VALUES (?, ?, ?)"
                    " ON CONFLICT(id) DO UPDATE SET verified_at = excluded.verified_at",
                    (qid, qid, now()),
                )

    def set_pending(self, qid: int, claim: str, reason: str) -> None:
        """Not sent yet (e.g. the agent is mid-turn): visible as pending, retried, not counted
        as a failed attempt. ``pending_since`` keeps the first time (the busy grace counts
        from it) until the answer is delivered."""
        with self.transaction():
            cur = self.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ? AND delivery_error = ?",
                                  (f"{PENDING}{reason}", qid, claim))
            if cur.rowcount:
                self.db.execute(
                    "INSERT INTO question_ext (id, group_id, pending_since) VALUES (?, ?, ?)"
                    " ON CONFLICT(id) DO UPDATE SET pending_since = COALESCE(pending_since, excluded.pending_since)",
                    (qid, qid, now()))

    def get_pending_since(self, qid: int) -> str | None:
        row = self.db.execute("SELECT pending_since FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return row[0] if row else None

    def record_session(self, session_id: str, transcript_path: str | None) -> None:
        if session_id and transcript_path:
            self.db.execute("INSERT OR REPLACE INTO sessions (session_id, transcript_path) VALUES (?, ?)",
                            (session_id, transcript_path))

    def session_transcript(self, session_id: str | None) -> str | None:
        if not session_id:
            return None
        row = self.db.execute("SELECT transcript_path FROM sessions WHERE session_id = ?", (session_id,)).fetchone()
        return row[0] if row else None

    def redeliver(self, qids: list[int]) -> list[int]:
        """Make answered questions pending delivery again (and available to check_answers)."""
        done = []
        for qid in qids:
            cur = self.db.execute(
                "UPDATE questions SET delivered_at = NULL, acknowledged_at = NULL, delivery_attempts = 0,"
                " delivery_error = ? WHERE id = ? AND status = 'answered'",
                (f"{PENDING}redelivery requested", qid),
            )
            if cur.rowcount:
                self.db.execute("UPDATE question_ext SET verified_at = NULL, pending_since = NULL WHERE id = ?", (qid,))
                done.append(qid)
        return done

    def _reconcile_unverified(self) -> None:
        """Re-check answers marked seen without a verified delivery, on every open.

        - Once (0.5.1 upgrade): every such answer from the last 7 days.
        - Always: rows with the signature of a pre-0.5.1 server, still running in sessions
          started before the update (``acknowledged_at = delivered_at``, set together on
          "send"), from the last 24 h.
        Each is looked up in its session's transcript: found → marked verified; not found
        (or no transcript) → un-seen, so check_answers returns it. A row is reset at most
        MAX_RESETS times, so an old server can never make this loop (and un-seen rows are not
        re-typed by anyone: they are already marked delivered).
        """
        one_off = not self.db.execute("SELECT 1 FROM meta WHERE key = ?", (RESET_KEY,)).fetchone()
        rows = self.db.execute(
            "SELECT q.id, q.session_id FROM questions q LEFT JOIN question_ext e ON e.id = q.id"
            " WHERE q.delivered_at IS NOT NULL AND q.acknowledged_at IS NOT NULL AND e.verified_at IS NULL"
            " AND COALESCE(e.reset_count, 0) < ?"
            " AND ((? AND q.answered_at >= ?) OR (q.acknowledged_at = q.delivered_at AND q.delivered_at >= ?))"
            " ORDER BY q.id LIMIT ?",
            (MAX_RESETS, int(one_off), now(-RESET_WINDOW), now(-SIGNATURE_WINDOW), RECONCILE_BATCH),
        ).fetchall()
        if not rows and not one_off:
            return
        from . import transcript  # stdlib only; kept local so the store has no import cycle

        by_session: dict[str | None, list[int]] = {}
        for qid, session in rows:
            by_session.setdefault(session, []).append(qid)
        found: set[int] = set()
        for session, qids in by_session.items():
            path = transcript.find(session, self.session_transcript(session))
            if path is not None:
                found |= transcript.landed_ids(path, qids)
        with self.transaction():
            for qid, _ in rows:
                if qid in found:
                    self.db.execute(
                        "INSERT INTO question_ext (id, group_id, verified_at) VALUES (?, ?, ?)"
                        " ON CONFLICT(id) DO UPDATE SET verified_at = excluded.verified_at", (qid, qid, now()))
                else:
                    self.db.execute("UPDATE questions SET acknowledged_at = NULL WHERE id = ?", (qid,))
                    self.db.execute(
                        "INSERT INTO question_ext (id, group_id, reset_count) VALUES (?, ?, 1)"
                        " ON CONFLICT(id) DO UPDATE SET reset_count = COALESCE(reset_count, 0) + 1", (qid, qid))
            if one_off and len(rows) < RECONCILE_BATCH:
                self.db.execute("INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)", (RESET_KEY, now()))

    def undelivered(self, pane_id: str, caps: dict[str, int]) -> list[Question]:
        """Answers for ``pane_id`` whose delivery failed and that the agent has not seen.

        An error starting with a key of ``caps`` is given up after that many attempts (the
        conversation is gone; the text was typed but never showed up); transient ones
        (pending, herdr errors) are retried indefinitely.
        """
        clauses = " OR ".join("(delivery_error LIKE ? AND delivery_attempts >= ?)" for _ in caps) or "0"
        args = [a for prefix, cap in caps.items() for a in (prefix + "%", cap)]
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE pane_id = ? AND status = 'answered' AND delivered_at IS NULL"
            " AND acknowledged_at IS NULL"
            # failed attempts, or never attempted (the answering process died) after a grace period
            " AND (delivery_error IS NOT NULL OR answered_at < ?)"
            f" AND NOT ({clauses}) ORDER BY id",
            (pane_id, now(-NEVER_ATTEMPTED_GRACE), *args),
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
            f"SELECT * FROM {VIEW} WHERE {where} AND status = 'answered' AND acknowledged_at IS NULL ORDER BY id",
            args,
        ).fetchall()
        return [Question.from_row(r) for r in rows]
