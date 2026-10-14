"""Remove old per-version MCP venvs from ${CLAUDE_PLUGIN_DATA}.

Each plugin version runs its MCP server from ``venv-<version>`` (0.5.3+; the shared ``venv``
before that). The data dir is kept across updates, so old ones pile up.

"In use" is answered by the kernel, not by matching paths in ``ps`` output: ``inbox mcp``
first thing (before any heavy import) takes a shared ``flock`` on ``<its venv>/.inbox-in-use``
and holds it for its lifetime (``claim``), refreshing the file's mtime; if its venv is gone
by then it exits instead of running from a deleted tree. Cleanup, on MCP server start,
considers only direct children of the data dir named ``venv-<numeric version>`` for a
version OLDER than its own, whose dir AND lock file are untouched for an hour, and removes
one only while holding an exclusive non-blocking flock on its lock file (so no server has
it), checked to still be the same file. Venvs without a lock file (servers from before the
lock, 0.5.2 and older, and the shared ``venv``) are never removed automatically: an mtime
says nothing about use. ``inbox clean-venvs`` removes them on request. Leftover
``.removing-*`` dirs (a cleanup killed mid-delete) older than an hour are swept.

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


def _version(text: str) -> tuple[int, ...] | None:
    if not re.fullmatch(r"\d+(\.\d+)*", text):
        return None
    return tuple(int(p) for p in text.split("."))


def claim(venv: Path):
    """Mark ``venv`` in use for as long as the returned file stays open (the MCP server keeps
    it for its lifetime), and refresh the lock file's mtime. If the venv was removed (a
    cleanup won the race before we got here), exit: never run from a deleted tree; Claude
    Code restarts the server and uv rebuilds the venv."""
    import sys

    lock = Path(venv) / LOCK_NAME
    try:
        fh = open(lock, "a")
        fcntl.flock(fh, fcntl.LOCK_SH)
        same = os.fstat(fh.fileno()).st_ino == os.stat(lock).st_ino
    except FileNotFoundError:
        same = False
    if not same:
        sys.stderr.write(f"herdr-inbox: venv {venv} was removed while starting; exiting so it is rebuilt\n")
        sys.exit(75)  # EX_TEMPFAIL
    try:
        os.utime(lock)
    except OSError:
        pass
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


def _try_remove(path: Path, lockless: bool) -> bool:
    lock = path / LOCK_NAME
    try:
        fd = os.open(lock, os.O_RDONLY)
    except FileNotFoundError:
        # a pre-lock server may still run from it: only on request (inbox clean-venvs)
        return lockless and path.is_dir() and _remove(path)
    except OSError:
        return False
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            return False  # a server holds it
        try:
            st = os.stat(lock)
            if os.fstat(fd).st_ino != st.st_ino:
                return False  # removed (and maybe recreated) meanwhile
            if time.time() - st.st_mtime < SETTLED_SECONDS:
                return False  # claimed by a server within the hour
        except OSError:
            return False
        return _remove(path)
    finally:
        os.close(fd)


def cleanup(data: Path, current: str, lockless: bool = False) -> list[Path]:
    """Remove the old, settled, unused venvs under ``data``; returns what was removed.
    ``lockless`` (``inbox clean-venvs``) also removes old venvs that have no lock file."""
    mine = _version(current)
    data = Path(data)
    if mine is None or not data.is_dir():
        return []
    removed = []
    for path in sorted(data.iterdir()):
        try:
            if not path.is_dir() or path.is_symlink():
                continue
            if path.name.startswith(".removing-venv"):  # a cleanup killed mid-delete
                if time.time() - path.stat().st_mtime >= SETTLED_SECONDS:
                    shutil.rmtree(path, ignore_errors=True)
                continue
            if path.name == "venv":
                older = True  # the shared venv every version before 0.5.3 used
            elif path.name.startswith("venv-") and (v := _version(path.name[5:])) is not None:
                older = v < mine
            else:
                continue
            age = time.time() - path.stat().st_mtime
            if older and age >= SETTLED_SECONDS and _try_remove(path, lockless):
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


def claim_current() -> None:
    """``inbox mcp``, before anything heavy: hold this venv's lock (exits if it is gone)."""
    import sys

    global _held
    if sys.prefix != sys.base_prefix:  # running from a venv
        try:
            _held = claim(Path(sys.prefix))
        except OSError as exc:  # e.g. a filesystem without flock: run anyway, cleanup fails safe
            log.warning("venv lock: %s", exc)


def start_cleanup() -> None:
    """Remove old venvs in the background; never fatal, the server never waits for it."""
    import threading

    def run() -> None:
        try:
            for path in cleanup_from_env():
                log.info("removed old venv %s", path)
        except Exception as exc:  # noqa: BLE001
            log.warning("old venv cleanup: %s", exc)

    threading.Thread(target=run, name="venv-cleanup", daemon=True).start()
