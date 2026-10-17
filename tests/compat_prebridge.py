"""Pre-bridge (<= 0.10.5) schema stamps and open behaviour, for the P0b revision tests.

Releases up to 0.10.5 stamped ``PRAGMA user_version`` with ``crc32(schema + Store._migrate
source) & 0x7FFFFFFF`` and re-migrated on every open whose stamp differed. The values below
were computed by importing ``src/herdr_inbox/store.py`` as it is at each commit
(``SCHEMA_STAMP``), not invented.
"""

from __future__ import annotations

import sqlite3

from herdr_inbox import store as st

LEGACY_STAMPS = {
    "0.10.3-0.10.5 (196504e..a9b215a)": 1668105336,
    "0.10.0 (ec16d20)": 2042562191,
    "migration under BEGIN IMMEDIATE (ff8cbed)": 51121320,
    "first stamped schema (3c0ddfb)": 2018474303,
    "0.10.5 fallback, no .py on disk ('revision 1')": 251294378,
}
CURRENT_PRE_BRIDGE = LEGACY_STAMPS["0.10.3-0.10.5 (196504e..a9b215a)"]


def pre_bridge_open(path, stamp: int = CURRENT_PRE_BRIDGE, timeout: float = 5.0) -> None:
    """What a 0.10.5 process does on ``Store()``: migrate under BEGIN IMMEDIATE unless the
    stamp is its own crc32, then stamp it (its schema is this code's: the bridge adds none)."""
    db = sqlite3.connect(path, timeout=timeout, isolation_level=None)
    try:
        if db.execute("PRAGMA user_version").fetchone()[0] == stamp:
            return
        db.execute("PRAGMA journal_mode=WAL")
        db.execute("BEGIN IMMEDIATE")
        try:
            if db.execute("PRAGMA user_version").fetchone()[0] == stamp:
                db.execute("COMMIT")
                return
            for statement in st._statements(st.SCHEMA):
                db.execute(statement)
            for table, added in st.TABLE_ADDED_COLUMNS.items():
                have = {r[1] for r in db.execute(f"PRAGMA table_info({table})")}
                for name, decl in added.items():
                    if name not in have:
                        db.execute(f"ALTER TABLE {table} ADD COLUMN {name} {decl}")
            for statement in st._statements(st.VIEW_SQL):
                db.execute(statement)
            db.execute(f"PRAGMA user_version = {stamp}")
            db.execute("COMMIT")
        except BaseException:
            db.execute("ROLLBACK")
            raise
    finally:
        db.close()


def stamp_of(path) -> int:
    db = sqlite3.connect(path)
    try:
        return db.execute("PRAGMA user_version").fetchone()[0]
    finally:
        db.close()


def set_stamp(path, value: int) -> None:
    db = sqlite3.connect(path, isolation_level=None)
    try:
        db.execute(f"PRAGMA user_version = {value}")
    finally:
        db.close()


def columns(path, table: str) -> set[str]:
    db = sqlite3.connect(path)
    try:
        return {r[1] for r in db.execute(f"PRAGMA table_info({table})")}
    finally:
        db.close()


def make_older_head(path, stamp: int = CURRENT_PRE_BRIDGE, table: str = "remote_outbox",
                    column: str = "sending_at") -> None:
    """A database a release before ``column`` existed migrated and stamped."""
    db = sqlite3.connect(path, isolation_level=None)
    try:
        db.execute(f"ALTER TABLE {table} DROP COLUMN {column}")
        db.execute(f"PRAGMA user_version = {stamp}")
    finally:
        db.close()


def traced(monkeypatch) -> list[str]:
    """Every SQL statement run on file connections opened from here on (sqlite3.connect is
    patched process-wide: open helper connections before calling this)."""
    statements: list[str] = []
    real_connect = sqlite3.connect

    def connect(*a, **k):
        db = real_connect(*a, **k)
        if (a[0] if a else k.get("database")) != ":memory:":  # store.reference_schema's scratch copy
            db.set_trace_callback(statements.append)
        return db

    monkeypatch.setattr(sqlite3, "connect", connect)
    return statements


def schema_statements(statements: list[str]) -> list[str]:
    """Schema work or a stamp write among ``statements``."""
    return [s for s in statements if s.lstrip().upper().startswith(
        ("CREATE", "ALTER", "DROP", "PRAGMA USER_VERSION =", "PRAGMA JOURNAL_MODE"))]
