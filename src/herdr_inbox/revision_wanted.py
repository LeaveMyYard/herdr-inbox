"""The "revision wanted" marker (herdr-inbox#50; Rust rewrite plan §5.4).

Only the worker and the popup migrate the database. A newer release whose hooks or MCP tools
find the database not migrated to their revision (SchemaNotReady) must not wait behind a worker
of an older revision that keeps the worker lock (its handover failed, or it never sees the
update): `worker.ensure()` does nothing while any worker holds the lock, so the newer processes
would stay "not ready" until the popup is opened.

So every non-migrating open that raises SchemaNotReady writes its revision here
(``<database dir>/revision-wanted``). A worker of a lower revision that finds the marker fresh,
naming a revision above its own and above the database's, releases the lock and exits; the next
``ensure()`` of the newer release (its MCP servers call it on SchemaNotReady) starts the newer
worker, which migrates. The marker counts only while it is rewritten (REVISION_WANTED_TTL): after
a rollback no newer process rewrites it, and older workers run again.

The Rust worker (P5) honours the same file. Standard library only (the hooks import this).
"""

from __future__ import annotations

import os
import time
from pathlib import Path

NAME = "revision-wanted"
# A marker older than this is stale: the newer processes that wrote it are gone (a rollback).
REVISION_WANTED_TTL = 600.0


def path_for(db: Path) -> Path:
    """The marker next to the database ``db``."""
    return Path(db).parent / NAME


def want(db: Path, revision: int) -> None:
    """A process of ``revision`` found ``db`` not migrated to it: say so (atomically; a newer
    mtime every time). Never lowers a higher revision a newer release wrote."""
    path = path_for(db)
    current = wanted(db)
    if current is not None and current > revision:
        revision = current
    tmp = path.with_name(f".{NAME}.{os.getpid()}.tmp")
    try:
        tmp.write_text(f"{revision}\n")
        os.replace(tmp, path)
    except OSError:
        try:
            tmp.unlink()
        except OSError:
            pass


def wanted(db: Path, now: float | None = None) -> int | None:
    """The revision a newer release wants ``db`` migrated to, or None (no marker, a stale one,
    or an unreadable one)."""
    path = path_for(db)
    try:
        age = (time.time() if now is None else now) - path.stat().st_mtime
        if age > REVISION_WANTED_TTL:
            return None
        return int(path.read_text().strip())
    except (OSError, ValueError):
        return None
