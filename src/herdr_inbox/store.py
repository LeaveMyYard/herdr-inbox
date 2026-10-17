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

from . import trace

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
    pending_since   TEXT,
    busy_attempts   INTEGER,
    notified_at     TEXT,
    herdr_server    TEXT,
    agent_gone_at   TEXT,
    legacy_misses   INTEGER,
    legacy_checked_at TEXT
);
-- Transcript path per Claude session, recorded by the SessionStart hook (0.5.1): delivery
-- is verified by finding the typed answer in it.
CREATE TABLE IF NOT EXISTS sessions (
    session_id      TEXT PRIMARY KEY,
    transcript_path TEXT,
    hook_delivery   TEXT,  -- written by 0.9.0 pre-release heads only; unused (columns are never dropped)
    hook_seen_at    TEXT,  -- the PostToolBatch/Stop delivery hooks fired (the WORKING gate)
    watch_seen_at   TEXT,  -- the asyncRewake watcher is waiting (the IDLE gate, with its lock)
    continued_in    TEXT   -- Claude Code continued this conversation in that session (0.10.3)
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
-- Remote machines (0.7, docs/REMOTE.md). On every machine: a change feed for `inbox export`
-- (filled by triggers, pruned by the worker) and the outcomes of `inbox apply` requests.
CREATE TABLE IF NOT EXISTS changes (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    qid             INTEGER NOT NULL
);
CREATE TRIGGER IF NOT EXISTS changes_q_ins AFTER INSERT ON questions BEGIN
    INSERT INTO changes (qid) VALUES (NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS changes_q_upd AFTER UPDATE ON questions BEGIN
    INSERT INTO changes (qid) VALUES (NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS changes_q_del AFTER DELETE ON questions BEGIN
    INSERT INTO changes (qid) VALUES (OLD.id);
END;
CREATE TRIGGER IF NOT EXISTS changes_x_ins AFTER INSERT ON question_ext BEGIN
    INSERT INTO changes (qid) VALUES (NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS changes_x_upd AFTER UPDATE ON question_ext BEGIN
    INSERT INTO changes (qid) VALUES (NEW.id);
END;
CREATE TABLE IF NOT EXISTS applied_requests (
    request_id      TEXT PRIMARY KEY,
    qid             INTEGER,
    outcome         TEXT NOT NULL,
    reason          TEXT,
    at              TEXT NOT NULL
);
-- Alerts (0.8): an agent's "please check X" with no answer expected, and automatic ones for
-- agents stuck at an approval/question prompt (herdr agent status `blocked`).
CREATE TABLE IF NOT EXISTS alerts (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at      TEXT NOT NULL,
    kind            TEXT NOT NULL,
    severity        TEXT NOT NULL,
    title           TEXT NOT NULL,
    message         TEXT,
    pane_id         TEXT,
    tab_id          TEXT,
    tab_label       TEXT,
    session_id      TEXT,
    asked_by        TEXT,
    herdr_server    TEXT,
    status          TEXT NOT NULL DEFAULT 'open',
    acked_at        TEXT,
    closed_at       TEXT,
    reported_at     TEXT
);
CREATE INDEX IF NOT EXISTS alerts_status ON alerts(status);
CREATE TABLE IF NOT EXISTS blocked_panes (
    herdr_server    TEXT NOT NULL,
    pane_id         TEXT NOT NULL,
    since           TEXT NOT NULL,
    PRIMARY KEY (herdr_server, pane_id)
);
CREATE TABLE IF NOT EXISTS remote_alerts (
    machine_id      TEXT NOT NULL,
    db_id           TEXT NOT NULL,
    alert_id        INTEGER NOT NULL,
    akey            TEXT NOT NULL,
    status          TEXT NOT NULL,
    data_json       TEXT NOT NULL,
    first_seen_at   TEXT NOT NULL,
    PRIMARY KEY (machine_id, alert_id)
);
-- On the operator's machine: the mirror of each remote machine and the actions queued for it.
CREATE TABLE IF NOT EXISTS remote_machines (
    machine_id      TEXT PRIMARY KEY,
    label           TEXT,
    target          TEXT,
    db_id           TEXT,
    cursor_seq      INTEGER,
    version         TEXT,
    uv              TEXT,
    herdr           TEXT,
    plugin_root     TEXT,
    pulls           INTEGER NOT NULL DEFAULT 0,
    failures        INTEGER NOT NULL DEFAULT 0,
    last_ok_at      TEXT,
    last_error      TEXT,
    last_try_at     TEXT,
    backoff_until   TEXT
);
CREATE TABLE IF NOT EXISTS remote_questions (
    machine_id      TEXT NOT NULL,
    db_id           TEXT NOT NULL,
    remote_id       INTEGER NOT NULL,
    qkey            TEXT NOT NULL,
    status          TEXT NOT NULL,
    data_json       TEXT NOT NULL,
    first_seen_at   TEXT NOT NULL,
    notified_at     TEXT,
    issue_notified_at TEXT,
    cleared_at      TEXT,
    PRIMARY KEY (machine_id, remote_id)
);
CREATE TABLE IF NOT EXISTS remote_outbox (
    request_id      TEXT PRIMARY KEY,
    machine_id      TEXT NOT NULL,
    db_id           TEXT NOT NULL,
    remote_id       INTEGER NOT NULL,
    qkey            TEXT NOT NULL,
    action          TEXT NOT NULL,
    detail_json     TEXT,
    created_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'queued',
    result          TEXT,
    sent_at         TEXT,
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_try_at     TEXT,
    sending_at      TEXT
);
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
EXT_ADDED_COLUMNS = {"options_json": "TEXT", "hidden_at": "TEXT", "verified_at": "TEXT", "reset_count": "INTEGER", "pending_since": "TEXT", "busy_attempts": "INTEGER", "notified_at": "TEXT", "herdr_server": "TEXT",
                     "agent_gone_at": "TEXT", "legacy_misses": "INTEGER", "legacy_checked_at": "TEXT",
                     "hook_sent_at": "TEXT", "hook_misses": "INTEGER"}  # question_ext is ours alone; 0.1.x never reads it

# Columns added to a table after it first shipped, per table: Store._migrate adds whichever an
# existing database lacks (tests/test_schema_migrations.py fails if a CREATE TABLE gains a
# column that is neither in its first-shipped set nor here).
TABLE_ADDED_COLUMNS = {
    "questions": ADDED_COLUMNS,
    "question_ext": EXT_ADDED_COLUMNS,
    "remote_questions": {"issue_notified_at": "TEXT", "cleared_at": "TEXT"},
    "remote_outbox": {"sending_at": "TEXT"},
    "sessions": {"hook_delivery": "TEXT", "hook_seen_at": "TEXT", "watch_seen_at": "TEXT", "continued_in": "TEXT"},
}


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


def _starts_dir() -> Path:
    return db_path().parent / "starts"


def record_start(session_id: str | None) -> None:
    """A Claude process started (or resumed, cleared into, compacted) ``session_id`` now: the
    SessionStart hook touches ``<state>/starts/<session>``. A file, not a column: no schema
    change, no database lock on the hook's path, and older versions simply ignore it."""
    from . import transcript  # stdlib only; local so the store has no import cycle

    sid = transcript.session_id_or_none(session_id)
    if sid is None:
        return
    d = _starts_dir()
    d.mkdir(parents=True, exist_ok=True)
    (d / sid).touch()  # an existing marker gets the new time too


STARTS_KEEP = 30 * 86400  # a start marker older than this is dropped (a missing one falls back)


def prune_starts(keep: float = STARTS_KEEP, clock=None) -> int:
    """Drop start markers older than ``keep`` seconds (the worker, at most hourly). A marker
    only matters while it is newer than its session's continued-in record; a session idle for
    longer than ``keep`` with its marker gone falls back to D6 plus the typed-once path."""
    import time as _time

    cutoff = (clock or _time.time)() - keep
    n = 0
    try:
        entries = list(_starts_dir().iterdir())
    except OSError:
        return 0
    for path in entries:
        try:
            if path.stat().st_mtime < cutoff:
                path.unlink()
                n += 1
        except OSError:
            continue
    return n


def started_at(session_id: str | None) -> float | None:
    """When a Claude process last started ``session_id`` (record_start), or None."""
    from . import transcript

    sid = transcript.session_id_or_none(session_id)
    if sid is None:
        return None
    try:
        return (_starts_dir() / sid).stat().st_mtime
    except OSError:
        return None


def live_session(reported: str | None, known: str | None = None) -> str | None:
    """The session a Claude process reported as ``reported`` (by herdr, or its own
    CLAUDE_CODE_SESSION_ID) runs now: ``transcript.follow``, except that a session a Claude
    process (re)started AFTER its continued-in record runs ITSELF. herdr resumes a pane under
    the OLD id, and until its first prompt the old transcript still ends in continued-in, so
    the D6 rule alone takes the older conversation for the newer one (live 2026-10-17: Q129-
    Q152 were typed into 1cd70de7, an ancestor of the asking c5240607)."""
    from . import transcript

    if not reported:
        return reported
    path = transcript.find(reported, known)
    if path is not None and transcript.continued_in(path) is not None:
        started, at = started_at(reported), transcript.continued_at(path)
        if started is not None and at is not None and started > at:
            return transcript.session_id_or_none(reported) or reported
    return transcript.follow(reported, known)


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
REQUEUE_WINDOW = timedelta(hours=1)
AGENT_GONE = "agent gone: "
REQUEUE_SETTLE = timedelta(seconds=20)  # > service.VERIFY_SECONDS: the record may still be on its way  # an old server's unlanded delivery this recent is sent again
REQUEUED = "pending: requeued: an earlier delivery never reached the conversation; sending when the agent is idle"
MAX_RESETS = 2
# A delivery claim older than this belongs to a process that died mid-send.
CLAIM_STALE = timedelta(seconds=60)
MAX_HOOK_MISSES = 2  # 0.9.0: after this many unseen hook sends an answer is left to typing


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


# The schema revision this code migrates to, stored as ``PRAGMA user_version`` (the P0b
# bridge, docs/plans/rust-rewrite.md §2.3 / §5.4 in herdr-panorama). Bump it whenever the
# schema or the migration changes; the Rust port shares the number.
MIGRATION_REVISION = 1

# Revision space. Releases up to 0.10.5 stamped ``crc32(schema + Store._migrate source) &
# 0x7FFFFFFF``: always 0..2**31-1, never negative. A revision is stored with the sign bit set
# (REVISION_BASE + revision, a negative int32), so a legacy crc32 -- however large -- can never
# be read as a revision, let alone as a higher one. Anything that is not a revision (a crc32
# left by a pre-bridge process, 0 on a fresh or pre-stamp database) is "unknown": the migrating
# processes migrate forward (idempotent) and stamp their revision.
REVISION_BASE = -(1 << 31)
MAX_REVISION = (1 << 31) - 1


def revision_stamp(revision: int | None = None) -> int:
    """The ``user_version`` that says "migrated to ``revision``" (default: this code's)."""
    revision = MIGRATION_REVISION if revision is None else revision
    if not 0 < revision <= MAX_REVISION:
        raise ValueError(f"revision out of range: {revision}")
    return REVISION_BASE + revision


def stamped_revision(user_version: int) -> int | None:
    """The revision a ``user_version`` names, or None for a legacy crc32 stamp or 0."""
    return user_version - REVISION_BASE if user_version < 0 else None


class SchemaNotReady(sqlite3.DatabaseError):
    """A non-migrating open found a database whose schema this code cannot use yet (not
    migrated, or migrated by an older release): the caller does nothing -- a hook exits 0, an
    MCP tool returns an "unavailable" receipt -- until the worker or the popup migrates it."""


def _apply_schema(db: sqlite3.Connection) -> None:
    """Every statement that brings a database to this code's schema; idempotent (CREATE IF NOT
    EXISTS, ADD COLUMN only when missing). The caller holds the transaction."""
    for statement in _statements(SCHEMA):
        db.execute(statement)
    for table, added in TABLE_ADDED_COLUMNS.items():
        if table == "questions":
            continue  # after the views, below (as before)
        have = {r[1] for r in db.execute(f"PRAGMA table_info({table})")}
        for name, decl in added.items():
            if name not in have:
                db.execute(f"ALTER TABLE {table} ADD COLUMN {name} {decl}")
    for old in OLD_VIEWS:
        db.execute(f"DROP VIEW IF EXISTS {old}")
    for statement in _statements(VIEW_SQL):
        db.execute(statement)
    columns = {r[1] for r in db.execute("PRAGMA table_info(questions)")}
    for name, decl in ADDED_COLUMNS.items():  # databases from before the column existed
        if name not in columns:
            db.execute(f"ALTER TABLE questions ADD COLUMN {name} {decl}")


def describe_schema(db: sqlite3.Connection) -> tuple[dict[str, list[tuple]], dict[str, tuple[str, str | None]]]:
    """(columns per table and view as (name, type, notnull, default, pk), and every object as
    name -> (type, whitespace-collapsed SQL; None for tables, whose columns say it all)),
    without SQLite's own objects."""
    objects = {name: (kind, None if kind == "table" or sql is None else " ".join(sql.split()))
               for kind, name, sql in db.execute("SELECT type, name, sql FROM sqlite_master")
               if not name.startswith("sqlite_")}
    columns = {name: [tuple(r[1:6]) for r in db.execute(f"PRAGMA table_info({name})")]
               for name, (kind, _) in objects.items() if kind in ("table", "view")}
    return columns, objects


def reference_schema() -> tuple[dict[str, list[tuple]], dict[str, tuple[str, str | None]]]:
    """``describe_schema`` of a fresh in-memory database migrated by this code: the schema it
    reads and writes, exact by construction (no parsing of SCHEMA)."""
    db = sqlite3.connect(":memory:")
    try:
        _apply_schema(db)
        return describe_schema(db)
    finally:
        db.close()


_expected_cache: tuple[dict[str, set[str]], set[str]] | None = None


def _expected_schema() -> tuple[dict[str, set[str]], set[str]]:
    """(columns per table, names of every other schema object) this code reads and writes;
    built once per process, only on the stale-stamp path."""
    global _expected_cache
    if _expected_cache is None:
        columns, objects = reference_schema()
        tables = {name: {c[0] for c in columns[name]} for name, (kind, _) in objects.items() if kind == "table"}
        _expected_cache = (tables, {name for name, (kind, _) in objects.items() if kind != "table"})
    return _expected_cache


def schema_missing(db: sqlite3.Connection) -> list[str]:
    """What of this code's schema the database lacks (tables, columns, indexes, triggers, the
    view); [] when it has everything. Read-only: two SELECTs, no lock beyond a read."""
    tables, others = _expected_schema()
    have: dict[str, set[str]] = {}
    for table, column in db.execute(
            "SELECT m.name, p.name FROM sqlite_master m JOIN pragma_table_info(m.name) p WHERE m.type = 'table'"):
        have.setdefault(table, set()).add(column)
    names = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type IN ('index', 'trigger', 'view')")}
    missing = [f"{t}.{c}" for t, cols in sorted(tables.items()) for c in sorted(cols - have.get(t, set()))]
    return missing + sorted(others - names)


def _statements(script: str) -> list[str]:
    """Split a SQL script into statements (so they can run inside one transaction:
    executescript would commit it)."""
    out, buf = [], ""
    for line in script.splitlines(keepends=True):
        buf += line
        if sqlite3.complete_statement(buf):
            if buf.strip():
                out.append(buf.strip())
            buf = ""
    if buf.strip():
        out.append(buf.strip())
    return out


def family(db, session_id: str | None, known=lambda s: None, limit: int = 32) -> list[str]:
    """``session_id`` plus its live predecessors (see Store.family); ``db`` may be read-only."""
    if not session_id:
        return []
    from . import transcript  # stdlib only; local so the store has no import cycle

    fam, frontier = [session_id], [session_id]
    while frontier and len(fam) < limit:
        current = frontier.pop()
        for (prev,) in db.execute("SELECT session_id FROM sessions WHERE continued_in = ?", (current,)).fetchall():
            if prev in fam:
                continue
            path = transcript.find(prev, known(prev))
            if path is not None and transcript.continued_in(path) == current:
                fam.append(prev)
                frontier.append(prev)
    return fam


class Store:
    def __init__(self, path: Path | str | None = None, timeout: float = 15.0, maintenance: bool = True,
                 *, migrate: bool = False):
        """``maintenance=False`` (MCP tool calls): skip the reconcile pass -- the delivery
        worker runs it every tick -- so a tool call never contends for the write lock with
        it.

        ``migrate=True`` only for the delivery worker and the popup: they bring an older
        database forward to MIGRATION_REVISION (never lowering a higher one). Every other open
        (hooks, the rewake watcher, MCP tools, the CLI, helper threads) never does schema work
        and never writes the stamp: on a database not stamped with this revision or a higher
        one it checks that the schema it uses is all there, read-only, and raises
        SchemaNotReady if not."""
        self.path = Path(path) if path else db_path()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(self.path, timeout=timeout, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        try:
            self.db.execute(f"PRAGMA busy_timeout={int(timeout * 1000)}")
            revision = stamped_revision(self.db.execute("PRAGMA user_version").fetchone()[0])
            if revision is None or revision < MIGRATION_REVISION:
                if migrate:
                    with trace.step("store.schema"):
                        self._migrate()
                else:
                    with trace.step("store.schema_check"):
                        if missing := schema_missing(self.db):
                            raise SchemaNotReady(f"inbox database not migrated yet: lacks {', '.join(missing[:5])}")
            if maintenance:
                with trace.step("store.reconcile"):
                    self._reconcile_unverified()
        except BaseException:
            self.db.close()
            raise

    def _migrate(self) -> None:
        """Bring the schema forward to MIGRATION_REVISION, once, under the write lock: several
        processes opening an old database together serialize here, and every one after the
        first finds the revision already stamped and does nothing. Idempotent (CREATE IF NOT
        EXISTS, ADD COLUMN only when missing), so a pre-bridge process re-stamping its crc32
        costs one more no-op pass, never data. A higher revision is left alone."""
        self.db.execute("PRAGMA journal_mode=WAL")  # cannot change inside a transaction
        self.db.execute("BEGIN IMMEDIATE")
        try:
            revision = stamped_revision(self.db.execute("PRAGMA user_version").fetchone()[0])
            if revision is not None and revision >= MIGRATION_REVISION:
                self.db.execute("COMMIT")  # someone migrated while we waited (or a newer release did)
                return
            _apply_schema(self.db)
            self.db.execute(f"PRAGMA user_version = {revision_stamp()}")
            self.db.execute("COMMIT")
        except BaseException:
            self.db.execute("ROLLBACK")
            raise

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
        if session_id:  # this session and the sessions it continued (Claude Code session continuation)
            fam = self.family(session_id)
            return f"session_id IN ({','.join('?' * len(fam))})", tuple(fam)
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
        """Open questions plus open alerts per pane (the ❓N badge and $inbox_q)."""
        rows = self.db.execute(
            "SELECT pane_id, COUNT(*) FROM ("
            " SELECT pane_id FROM questions WHERE status = 'open' AND pane_id IS NOT NULL"
            " UNION ALL SELECT pane_id FROM alerts WHERE status = 'open' AND pane_id IS NOT NULL"
            ") GROUP BY pane_id"
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
            if cur.rowcount and reason.startswith("busy:"):  # the grace counts from the first busy moment
                self.db.execute(
                    "INSERT INTO question_ext (id, group_id, pending_since) VALUES (?, ?, ?)"
                    " ON CONFLICT(id) DO UPDATE SET pending_since = COALESCE(pending_since, excluded.pending_since)",
                    (qid, qid, now()))

    def record_busy_miss(self, qid: int, claim: str, reason: str) -> None:
        """Typed while the agent was busy but not seen in the transcript: back to pending
        (not a failed attempt, so the idle path keeps its full cap), counted separately."""
        with self.transaction():
            cur = self.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ? AND delivery_error = ?",
                                  (f"{PENDING}{reason}", qid, claim))
            if cur.rowcount:
                self.db.execute(
                    "INSERT INTO question_ext (id, group_id, busy_attempts) VALUES (?, ?, 1)"
                    " ON CONFLICT(id) DO UPDATE SET busy_attempts = COALESCE(busy_attempts, 0) + 1", (qid, qid))

    def busy_attempts(self, qid: int) -> int:
        row = self.db.execute("SELECT busy_attempts FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return (row[0] or 0) if row else 0

    def get_pending_since(self, qid: int) -> str | None:
        row = self.db.execute("SELECT pending_since FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return row[0] if row else None

    def record_session(self, session_id: str, transcript_path: str | None) -> None:
        """Where the session's transcript is (kept on conflict: hook_seen_at lives here too).
        ``sessions.hook_delivery`` is no longer read or written: liveness is ``hook_seen_at``."""
        if session_id and transcript_path:
            self.db.execute(
                "INSERT INTO sessions (session_id, transcript_path) VALUES (?, ?)"
                " ON CONFLICT(session_id) DO UPDATE SET transcript_path = excluded.transcript_path",
                (session_id, transcript_path))

    def continued_in(self, session_id: str | None) -> str | None:
        if not session_id:
            return None
        row = self.db.execute("SELECT continued_in FROM sessions WHERE session_id = ?", (session_id,)).fetchone()
        return row[0] if row else None

    def record_continuation(self, old: str, new: str) -> None:
        """Claude Code continued ``old``'s conversation in ``new``. Rows keep the session they
        were FILED under (it is part of a remote question's identity, qkey; review #41): who
        owns them, where they are delivered and verified is resolved through this record."""
        if not old or not new or old == new or self.continued_in(old) == new:
            return
        self.db.execute("INSERT INTO sessions (session_id, continued_in) VALUES (?, ?)"
                        " ON CONFLICT(session_id) DO UPDATE SET continued_in = excluded.continued_in", (old, new))

    def family(self, session_id: str | None) -> list[str]:
        """``session_id`` and every session whose conversation continued into it.

        The stored ``continued_in`` only nominates a predecessor; it counts while its
        transcript STILL ends in ``continued-in`` to this session (review #41 final pass: a
        resumed old session owns its rows again, never stale). The read is cached and
        incremental (transcript.continued_in)."""
        return family(self.db, session_id, self.session_transcript)

    def watch_live(self, session_id: str | None, within: timedelta) -> bool:
        """The session's asyncRewake watcher stamped ``watch_seen_at`` within ``within``.
        Kept apart from ``hook_seen_at`` (#30 M1): a live watcher says nothing about the
        PostToolBatch/Stop hooks firing, so it never holds a WORKING session's answers."""
        if not session_id:
            return False
        row = self.db.execute("SELECT watch_seen_at FROM sessions WHERE session_id = ?", (session_id,)).fetchone()
        return bool(row and row[0] and row[0] >= now(-within))

    def hook_live(self, session_id: str | None, within: timedelta) -> bool:
        """Did this session's delivery hook fire within ``within``? Liveness, not capability
        (review #26 M1): a session whose hooks stopped firing (plugin updated or disabled
        mid-session, no PostToolBatch) falls back to the 0.8 busy rules."""
        if not session_id:
            return False
        row = self.db.execute("SELECT hook_seen_at FROM sessions WHERE session_id = ?", (session_id,)).fetchone()
        return bool(row and row[0] and row[0] >= now(-within))

    # -- hook delivery (0.9.0; DESIGN.md "Hook delivery to busy Claude sessions") ----------

    def hook_misses(self, qid: int) -> int:
        row = self.db.execute("SELECT hook_misses FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return (row[0] or 0) if row else 0

    def hook_candidates(self, session_id: str, max_misses: int) -> list[Question]:
        """This session's answers (and those of the sessions it continued) not yet delivered
        or seen, that the hook may still carry."""
        fam = self.family(session_id)
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE session_id IN ({','.join('?' * len(fam))}) AND status = 'answered'"
            " AND delivered_at IS NULL AND acknowledged_at IS NULL AND id NOT IN (SELECT id FROM question_ext"
            " WHERE COALESCE(hook_misses, 0) >= ? OR agent_gone_at IS NOT NULL) ORDER BY id",
            (*fam, max_misses)).fetchall()
        return [Question.from_row(r) for r in rows]

    def record_hook_delivery(self, qid: int, claim: str) -> bool:
        """Under our claim: handed to Claude Code as hook context (sent, not yet seen)."""
        with self.transaction():
            cur = self.db.execute("UPDATE questions SET delivered_at = ?, delivery_error = NULL"
                                  " WHERE id = ? AND delivery_error = ?", (now(), qid, claim))
            if cur.rowcount != 1:
                return False
            self.db.execute(
                "INSERT INTO question_ext (id, group_id, hook_sent_at) VALUES (?, ?, ?)"
                " ON CONFLICT(id) DO UPDATE SET hook_sent_at = excluded.hook_sent_at, pending_since = NULL",
                (qid, qid, now()))
            return True

    def hook_unverified(self, server: str) -> list[tuple[Question, str]]:
        """Hook-sent, not yet verified rows of this herdr server (or of none recorded)."""
        rows = self.db.execute(
            f"SELECT v.*, e.hook_sent_at AS _hook_sent_at FROM {VIEW} v JOIN question_ext e ON e.id = v.id"
            " WHERE e.hook_sent_at IS NOT NULL AND e.verified_at IS NULL AND v.delivered_at IS NOT NULL"
            " AND COALESCE(e.herdr_server, ?) = ? ORDER BY v.id", (server, server)).fetchall()
        return [(Question.from_row(r), r["_hook_sent_at"]) for r in rows]

    def hook_verified(self, qid: int, acknowledge: bool) -> None:
        with self.transaction():
            if acknowledge:
                self.db.execute("UPDATE questions SET acknowledged_at = COALESCE(acknowledged_at, ?) WHERE id = ?",
                                (now(), qid))
            self.db.execute("UPDATE question_ext SET verified_at = ? WHERE id = ?", (now(), qid))

    def hook_unseen(self, qid: int, sent_at: str, reason: str | None) -> bool:
        """Not in the transcript in time: back to pending (``reason``), a miss counted; with
        ``reason`` None (no transcript to look in), left sent-unverified for check_answers.
        Compare-and-set on the send that was judged (``sent_at``): a stale read never wipes
        a newer send (review #26 L1)."""
        with self.transaction():
            cur = self.db.execute("UPDATE question_ext SET hook_sent_at = NULL,"
                                  " hook_misses = COALESCE(hook_misses, 0) + ? WHERE id = ? AND hook_sent_at = ?",
                                  (1 if reason is not None else 0, qid, sent_at))
            if cur.rowcount != 1:
                return False
            if reason is not None:
                self.db.execute("UPDATE questions SET delivered_at = NULL, delivery_error = ?"
                                " WHERE id = ? AND acknowledged_at IS NULL", (f"{PENDING}{reason}", qid))
            return True

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
                self.db.execute("UPDATE question_ext SET verified_at = NULL, pending_since = NULL, busy_attempts = NULL WHERE id = ?", (qid,))
                done.append(qid)
        return done

    def _reconcile_unverified(self) -> None:
        """Re-check answers marked seen without a verified delivery, on every open.

        - Once (0.5.1 upgrade): every such answer from the last 7 days.
        - Always: rows with the signature of a pre-0.5.1 server, still running in sessions
          started before the update (``acknowledged_at = delivered_at``, set together on
          "send"), from the last 24 h.
        A row is looked at only once its delivery is REQUEUE_SETTLE old (Claude Code writes
        the transcript record a moment after the old server's DB write). Each is looked up in
        its session's transcript: found → marked verified; not found → un-seen, so
        check_answers returns it. Only when the transcript was actually read, the delivery
        was an old server's "typed = seen" (``acknowledged_at = delivered_at``) and is less
        than REQUEUE_WINDOW old, is it also put back for delivery (REQUEUED: at idle/done,
        and deliver_batch checks the transcript once more before typing). Without a
        transcript it is only un-seen, never retyped blind. A row is reset at most
        MAX_RESETS times, so an old server can never make this loop.
        """
        one_off = not self.db.execute("SELECT 1 FROM meta WHERE key = ?", (RESET_KEY,)).fetchone()
        rows = self.db.execute(
            "SELECT q.id, q.session_id, (q.acknowledged_at = q.delivered_at AND q.delivered_at >= ?)"
            " FROM questions q LEFT JOIN question_ext e ON e.id = q.id"
            " WHERE q.delivered_at IS NOT NULL AND q.acknowledged_at IS NOT NULL AND e.verified_at IS NULL"
            " AND COALESCE(e.reset_count, 0) < ? AND q.delivered_at <= ?"
            " AND ((? AND q.answered_at >= ?) OR (q.acknowledged_at = q.delivered_at AND q.delivered_at >= ?))"
            " ORDER BY q.id LIMIT ?",
            (now(-REQUEUE_WINDOW), MAX_RESETS, now(-REQUEUE_SETTLE), int(one_off), now(-RESET_WINDOW),
             now(-SIGNATURE_WINDOW),
             RECONCILE_BATCH),
        ).fetchall()
        if not rows and not one_off:
            return
        from . import transcript  # stdlib only; kept local so the store has no import cycle

        by_session: dict[str | None, list[int]] = {}
        for qid, session, _ in rows:
            by_session.setdefault(session, []).append(qid)
        found: set[int] = set()
        read: set[int] = set()  # rows whose transcript was actually read
        for session, qids in by_session.items():
            from .service import expected_segments  # local: service imports the store

            qs = [q for q in (self.get(i) for i in qids) if q is not None]
            for s in transcript.chain(session, self.session_transcript(session)):  # incl. continuations
                path = transcript.find(s, self.session_transcript(s))
                if path is not None:
                    found |= transcript.landed_exact(path, {q.id: (expected_segments(q), q.answered_at) for q in qs})
                    read |= set(qids)
        with self.transaction():
            for qid, _, recent_signature in rows:
                if qid in found:
                    self.db.execute(
                        "INSERT INTO question_ext (id, group_id, verified_at) VALUES (?, ?, ?)"
                        " ON CONFLICT(id) DO UPDATE SET verified_at = excluded.verified_at", (qid, qid, now()))
                elif recent_signature and qid in read:
                    # an old server typed it and called it seen; it never landed: deliver it
                    # again (the worker waits for idle; claims keep old servers from doubling)
                    self.db.execute("UPDATE questions SET acknowledged_at = NULL, delivered_at = NULL,"
                                    " delivery_error = ? WHERE id = ?", (REQUEUED, qid))
                    self.db.execute(
                        "INSERT INTO question_ext (id, group_id, reset_count) VALUES (?, ?, 1)"
                        " ON CONFLICT(id) DO UPDATE SET reset_count = COALESCE(reset_count, 0) + 1", (qid, qid))
                else:
                    self.db.execute("UPDATE questions SET acknowledged_at = NULL WHERE id = ?", (qid,))
                    self.db.execute(
                        "INSERT INTO question_ext (id, group_id, reset_count) VALUES (?, ?, 1)"
                        " ON CONFLICT(id) DO UPDATE SET reset_count = COALESCE(reset_count, 0) + 1", (qid, qid))
            if one_off and len(rows) < RECONCILE_BATCH:
                self.db.execute("INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)", (RESET_KEY, now()))

    def undelivered_all(self, caps: dict[str, int]) -> list[Question]:
        """``undelivered`` for every pane (the delivery worker)."""
        panes = [r[0] for r in self.db.execute(
            f"SELECT DISTINCT pane_id FROM {VIEW} WHERE pane_id IS NOT NULL AND status = 'answered'"
            " AND delivered_at IS NULL AND acknowledged_at IS NULL")]
        return [q for pane in panes for q in self.undelivered(pane, caps)]

    def stuck_answers(self) -> list[Question]:
        """Answered, never delivered (or delivered unverified) and not seen: candidates for
        the 'not delivered' notification."""
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE status = 'answered' AND acknowledged_at IS NULL"
            " AND verified_at IS NULL AND pane_id IS NOT NULL"
            # sent where no transcript could be found: most likely landed, not 'not delivered'
            " AND NOT (delivered_at IS NOT NULL AND delivery_error IS NULL) ORDER BY id").fetchall()
        return [Question.from_row(r) for r in rows]

    def mark_agent_gone(self, qid: int, reason: str) -> None:
        """The asking agent is gone (pane closed, conversation replaced): stop retrying; the
        answer stays unseen for check_answers (and the compact/resume hook)."""
        with self.transaction():
            self.db.execute("UPDATE question_ext SET agent_gone_at = COALESCE(agent_gone_at, ?) WHERE id = ?",
                            (now(), qid))
            self.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ? AND delivered_at IS NULL"
                            " AND COALESCE(delivery_error, '') NOT LIKE 'sending:%'", (f"{AGENT_GONE}{reason}", qid))

    def move_to_pane(self, qid: int, pane_id: str, tab_id: str | None, reason: str, claim: str | None = None,
                     tab_label: str | None = None) -> bool:
        """The asking conversation now runs in ``pane_id`` (its pane moved): deliver there.
        Under ``claim`` when one is held; clears an earlier 'agent gone'."""
        mine, args = (" AND delivery_error = ?", (claim,)) if claim else ("", ())
        with self.transaction():
            cur = self.db.execute(
                f"UPDATE questions SET pane_id = ?, tab_id = COALESCE(?, tab_id), tab_label = COALESCE(?, tab_label),"
                f" delivery_error = ?, delivery_attempts = 0"
                f" WHERE id = ? AND status = 'answered' AND acknowledged_at IS NULL{mine}",
                (pane_id, tab_id, tab_label, f"{PENDING}{reason}", qid, *args))
            if cur.rowcount:
                self.db.execute("UPDATE question_ext SET agent_gone_at = NULL WHERE id = ?", (qid,))
            return cur.rowcount == 1

    def gone_recent(self, within: timedelta) -> list[Question]:
        """Answers marked 'agent gone' recently, still unseen: maybe their pane only moved."""
        rows = self.db.execute(
            f"SELECT v.* FROM {VIEW} v JOIN question_ext e ON e.id = v.id WHERE e.agent_gone_at >= ?"
            " AND v.status = 'answered' AND v.acknowledged_at IS NULL AND v.delivered_at IS NULL"
            " AND v.session_id IS NOT NULL"
            " AND e.hidden_at IS NULL"  # the operator hid it: dealt with, never typed later (#42)
            " AND COALESCE(v.delivery_error, '') NOT LIKE 'sending:%'"  # a send in flight (e.g. Send now)
            " ORDER BY v.id", (now(-within),)).fetchall()
        return [Question.from_row(r) for r in rows]

    def agent_gone(self, qid: int) -> bool:
        row = self.db.execute("SELECT agent_gone_at FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return bool(row and row[0])

    def gone_candidates(self, prefixes: tuple[str, ...], cap: int) -> list[Question]:
        """Undelivered answers whose 'no agent' / 'conversation changed' retries ran out."""
        clauses = " OR ".join("delivery_error LIKE ?" for _ in prefixes)
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE status = 'answered' AND delivered_at IS NULL AND acknowledged_at IS NULL"
            f" AND ({clauses}) AND delivery_attempts >= ?"
            " AND id NOT IN (SELECT id FROM question_ext WHERE agent_gone_at IS NOT NULL) ORDER BY id",
            (*(p + "%" for p in prefixes), cap)).fetchall()
        return [Question.from_row(r) for r in rows]

    def legacy_check_due(self, qid: int, every: timedelta) -> bool:
        row = self.db.execute("SELECT legacy_checked_at FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return not (row and row[0] and row[0] > now(-every))

    def legacy_miss(self, qid: int) -> int:
        """Count one more 'that pane does not hold the asking conversation here'."""
        self.db.execute("UPDATE question_ext SET legacy_misses = COALESCE(legacy_misses, 0) + 1,"
                        " legacy_checked_at = ? WHERE id = ?", (now(), qid))
        row = self.db.execute("SELECT legacy_misses FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return row[0] if row else 0

    def set_server(self, qid: int, server: str | None) -> None:
        """The herdr server (its socket) the question was asked on: only that server's
        delivery worker types it."""
        self.db.execute("UPDATE question_ext SET herdr_server = ? WHERE id = ?", (server, qid))

    def server_of(self, qid: int) -> str | None:
        row = self.db.execute("SELECT herdr_server FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return row[0] if row else None

    def notified(self, qid: int) -> bool:
        row = self.db.execute("SELECT notified_at FROM question_ext WHERE id = ?", (qid,)).fetchone()
        return bool(row and row[0])

    def mark_notified(self, qids: list[int]) -> None:
        for qid in qids:
            self.db.execute(
                "INSERT INTO question_ext (id, group_id, notified_at) VALUES (?, ?, ?)"
                " ON CONFLICT(id) DO UPDATE SET notified_at = excluded.notified_at", (qid, qid, now()))

    def reconcile(self) -> None:
        """Re-check unverified acknowledgements now (the delivery worker, every tick)."""
        self._reconcile_unverified()

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
            # its agent is gone: left for check_answers, not retried (hiding a row only takes
            # it out of the popup -- a hidden pending answer is still delivered)
            " AND id NOT IN (SELECT id FROM question_ext WHERE agent_gone_at IS NOT NULL)"
            f" AND NOT ({clauses}) ORDER BY id",
            (pane_id, now(-NEVER_ATTEMPTED_GRACE), *args),
        ).fetchall()
        return [Question.from_row(r) for r in rows]

    def acknowledge(self, qids: list[int]) -> None:
        """The agent has the answer (check_answers): seen, and no longer a failed delivery --
        a stale error on a seen answer is noise (operator 2026-10-17)."""
        self.db.executemany(
            "UPDATE questions SET acknowledged_at = COALESCE(acknowledged_at, ?),"
            " delivery_error = CASE WHEN delivery_error LIKE 'sending:%' THEN delivery_error ELSE NULL END"
            " WHERE id = ?",
            [(now(), q) for q in qids],
        )

    def clear_seen_errors(self) -> int:
        """Reconcile: answers already seen (acknowledged) that still carry a delivery error."""
        return self.db.execute(
            "UPDATE questions SET delivery_error = NULL WHERE acknowledged_at IS NOT NULL"
            " AND delivery_error IS NOT NULL AND delivery_error NOT LIKE 'sending:%'").rowcount

    def archive_gone(self, after: timedelta) -> int:
        """Answers whose agent has been gone longer than ``after`` leave the popup by themselves
        (hidden; check_answers still returns them). Operator Q167, 2026-10-17."""
        return self.db.execute(
            "UPDATE question_ext SET hidden_at = ? WHERE agent_gone_at IS NOT NULL AND agent_gone_at <= ?"
            " AND hidden_at IS NULL AND id IN (SELECT id FROM questions WHERE status = 'answered'"
            " AND acknowledged_at IS NULL)", (now(), now(-after))).rowcount

    def failed_pile(self, within: timedelta) -> list[Question]:
        """Unseen answers shown (or about to show) as failed: not verified, agent gone,
        conversation changed, no agent -- reconcile candidates."""
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE status = 'answered' AND acknowledged_at IS NULL AND delivered_at IS NULL"
            " AND answered_at >= ? AND delivery_error IS NOT NULL AND delivery_error NOT LIKE 'sending:%'"
            " AND delivery_error NOT LIKE ? ORDER BY id", (now(-within), f"{PENDING}%")).fetchall()
        return [Question.from_row(r) for r in rows]

    def mark_seen_elsewhere(self, qid: int, claim: str | None = None) -> bool:
        """The agent has the answer (it read it some other way): seen, never typed again, no
        longer a failure. Not 'delivered': we did not deliver it."""
        with self.transaction():
            cur = self.db.execute(
                "UPDATE questions SET acknowledged_at = COALESCE(acknowledged_at, ?), delivery_error = NULL"
                " WHERE id = ? AND (delivery_error IS ? OR COALESCE(delivery_error, '') NOT LIKE 'sending:%')",
                (now(), qid, claim))
            if cur.rowcount:
                self.db.execute("UPDATE question_ext SET agent_gone_at = NULL WHERE id = ?", (qid,))
            return cur.rowcount == 1

    def mark_landed(self, qid: int, acknowledge: bool) -> bool:
        """Found in a transcript after all: delivered and verified (and seen unless only the
        long-answer reference landed); the error and any 'agent gone' are cleared."""
        with self.transaction():
            cur = self.db.execute(
                "UPDATE questions SET delivered_at = COALESCE(delivered_at, ?), delivery_error = NULL,"
                " acknowledged_at = CASE WHEN ? THEN COALESCE(acknowledged_at, ?) ELSE acknowledged_at END"
                " WHERE id = ? AND COALESCE(delivery_error, '') NOT LIKE 'sending:%'",
                (now(), int(acknowledge), now(), qid))
            if cur.rowcount:
                self.db.execute(
                    "INSERT INTO question_ext (id, group_id, verified_at) VALUES (?, ?, ?)"
                    " ON CONFLICT(id) DO UPDATE SET verified_at = COALESCE(verified_at, excluded.verified_at),"
                    " agent_gone_at = NULL", (qid, qid, now()))
            return cur.rowcount == 1

    def unacknowledged_answers(self, session_id: str | None, pane_id: str | None) -> list[Question]:
        where, args = self._owner_clause(session_id, pane_id)
        rows = self.db.execute(
            f"SELECT * FROM {VIEW} WHERE {where} AND status = 'answered' AND acknowledged_at IS NULL ORDER BY id",
            args,
        ).fetchall()
        return [Question.from_row(r) for r in rows]

