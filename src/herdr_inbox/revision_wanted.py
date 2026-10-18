"""The "revision wanted" marker (herdr-inbox#50; Rust rewrite plan §5.4).

Only the worker and the popup migrate the database. A newer release whose hooks or MCP tools find
the database not migrated to their revision (SchemaNotReady) writes the revision it wants to
``<database>.revision-wanted``. A worker of an older revision that sees it fresh, with no newer
worker standing by, starts the newer release's worker (herdr's current plugin root), which then
stands by until the older worker hands the lock over (worker_handover.py). The marker is how a
process that cannot start a worker itself (a Rust hook) asks for one.

Format, shared with the Rust store (``src/store/revision_wanted.rs``): one line,
``<revision> <unix seconds of the write>``. A marker counts while it is younger than
REVISION_WANTED_TTL (newer processes rewrite it while they stay not ready) and not dated more
than CLOCK_SKEW in the future (a clock step back must not keep it alive). Writes hold
``<database>.revision-wanted.lock`` (flock), so a higher revision is never lowered.

Standard library only (the hooks import this).
"""

from __future__ import annotations

import fcntl
import os
import time
from pathlib import Path

SUFFIX = ".revision-wanted"
# A marker older than this is stale: the newer processes that wrote it are gone (a rollback).
REVISION_WANTED_TTL = 600.0
# A marker dated further in the future than this is not trusted (the clock went back).
CLOCK_SKEW = 60.0


def path_for(db: Path) -> Path:
    """The marker of the database ``db`` (``inbox.db`` -> ``inbox.db.revision-wanted``)."""
    db = Path(db)
    return db.with_name(db.name + SUFFIX)


def _read(path: Path) -> tuple[int, float] | None:
    try:
        revision, written = path.read_text().split()
        return int(revision), float(written)
    except (OSError, ValueError):
        return None


def want(db: Path, revision: int, now: float | None = None) -> None:
    """A process of ``revision`` found ``db`` not migrated to it: say so, under the marker's lock,
    never lowering a higher revision that is still fresh."""
    path = path_for(db)
    at = time.time() if now is None else now
    try:
        with open(path.with_name(path.name + ".lock"), "a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            current = wanted(db, now=at)
            if current is not None and current > revision:
                return  # a fresher, higher request stands; never lower it, never extend it
            tmp = path.with_name(f".{path.name}.{os.getpid()}.tmp")
            tmp.write_text(f"{revision} {int(at)}\n")
            os.replace(tmp, path)
    except OSError:
        pass


def wanted(db: Path, now: float | None = None) -> int | None:
    """The revision a newer release wants ``db`` migrated to, or None (no marker, a stale one, one
    dated in the future, or an unreadable one)."""
    got = _read(path_for(db))
    if got is None:
        return None
    revision, written = got
    age = (time.time() if now is None else now) - written
    if age > REVISION_WANTED_TTL or age < -CLOCK_SKEW:
        return None
    return revision
