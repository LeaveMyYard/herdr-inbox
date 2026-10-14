"""Remove old per-version MCP venvs from ${CLAUDE_PLUGIN_DATA}.

Each plugin version runs its MCP server from ``venv-<version>`` (0.5.3+; the shared ``venv``
before that). The data dir is kept across updates, so old ones pile up.

"In use" is answered by the kernel, not by matching paths in ``ps`` output: every MCP
server holds a shared ``flock`` on ``<its venv>/.inbox-in-use`` for its lifetime
(``hold_lock``). Cleanup, on MCP server start, considers only direct children of the data
dir named ``venv`` or ``venv-<numeric version>`` for a version OLDER than its own and
untouched for an hour, and removes one only:

- with a lock file: while holding an exclusive non-blocking flock on it (so no server has
  it), checked to still be the same file;
- without one (servers from before the lock, 0.5.2 and older): if untouched for 7 days.

The venv is first renamed to a unique ``.removing-…`` name (atomic: concurrent cleanups
cannot both win), then deleted. Path spelling (symlinked data dir, relative paths) does not
matter, since the lock is on the file itself.
"""

from __future__ import annotations

import fcntl
import logging
import os
import re
import shutil
import time
import uuid
from pathlib import Path

log = logging.getLogger("herdr_inbox")

LOCK_NAME = ".inbox-in-use"
SETTLED_SECONDS = 3600
LOCKLESS_SETTLED_SECONDS = 7 * 86400


def _version(text: str) -> tuple[int, ...] | None:
    if not re.fullmatch(r"\d+(\.\d+)*", text):
        return None
    return tuple(int(p) for p in text.split("."))


def hold_lock(venv: Path):
    """Mark ``venv`` in use for as long as the returned file stays open (the MCP server
    keeps it for its lifetime)."""
    fh = open(Path(venv) / LOCK_NAME, "a")
    fcntl.flock(fh, fcntl.LOCK_SH)
    return fh


def _remove(path: Path) -> bool:
    """Rename ``path`` away and delete it; False if someone else got there first."""
    trash = path.with_name(f".removing-{path.name}-{uuid.uuid4().hex[:8]}")
    try:
        os.rename(path, trash)
    except OSError:
        return False
    shutil.rmtree(trash, ignore_errors=True)
    return True


def _try_remove(path: Path, age: float) -> bool:
    lock = path / LOCK_NAME
    try:
        fd = os.open(lock, os.O_RDONLY)
    except FileNotFoundError:
        if not path.is_dir() or age < LOCKLESS_SETTLED_SECONDS:
            return False  # a pre-lock server may still run from it
        return _remove(path)
    except OSError:
        return False
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            return False  # a server holds it
        try:
            if os.fstat(fd).st_ino != os.stat(lock).st_ino:
                return False  # removed (and maybe recreated) meanwhile
        except OSError:
            return False
        return _remove(path)
    finally:
        os.close(fd)


def cleanup(data: Path, current: str) -> list[Path]:
    """Remove the old, settled, unused venvs under ``data``; returns what was removed."""
    mine = _version(current)
    data = Path(data)
    if mine is None or not data.is_dir():
        return []
    removed = []
    for path in sorted(data.iterdir()):
        try:
            if not path.is_dir() or path.is_symlink():
                continue
            if path.name == "venv":
                older = True  # the shared venv every version before 0.5.3 used
            elif path.name.startswith("venv-") and (v := _version(path.name[5:])) is not None:
                older = v < mine
            else:
                continue
            age = time.time() - path.stat().st_mtime
            if older and age >= SETTLED_SECONDS and _try_remove(path, age):
                removed.append(path)
        except OSError as exc:  # vanished meanwhile (a concurrent cleanup), permissions...
            log.debug("old venv %s: %s", path, exc)
    return removed


def cleanup_from_env() -> list[Path]:
    """The MCP server's start-up call: ${CLAUDE_PLUGIN_DATA} and this package's version."""
    from importlib.metadata import PackageNotFoundError, version

    data = os.environ.get("CLAUDE_PLUGIN_DATA")
    if not data:
        return []
    try:
        current = version("herdr-inbox")
    except PackageNotFoundError:
        return []
    return cleanup(Path(data), current)


_held = None  # the server's own lock, open for the process lifetime


def start(prefix: str | None = None) -> None:
    """MCP server start: hold this venv's lock, then clean up old ones in the background.
    Never fatal, and the server never waits for the cleanup."""
    import sys
    import threading

    global _held
    prefix = prefix or sys.prefix
    if prefix != sys.base_prefix:  # running from a venv
        try:
            _held = hold_lock(Path(prefix))
        except OSError as exc:
            log.warning("venv lock: %s", exc)

    def run() -> None:
        try:
            for path in cleanup_from_env():
                log.info("removed old venv %s", path)
        except Exception as exc:  # noqa: BLE001
            log.warning("old venv cleanup: %s", exc)

    threading.Thread(target=run, name="venv-cleanup", daemon=True).start()
