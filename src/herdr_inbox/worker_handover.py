"""Handing the worker lock to a newer revision (herdr-inbox#50; Rust rewrite plan §5.4).

One worker per herdr server holds ``worker-<key>.lock``; only it (and the popup) migrates. After
a schema change a worker of an older revision may still hold the lock (its version handover
failed, or it never sees the update). The newer revision must take over WITHOUT a moment in which
nobody delivers, so the older worker never just exits: it hands over to a newer worker that is
already alive and waiting.

- **Holder record** ``worker-<key>.lock.holder``: ``<revision> <pid>``, written by the worker that
  holds the lock. A worker from before this record (0.10.6 and older) is revision 1.
- **Standby** ``worker-<key>.lock.standby``: ``<revision> <pid> <unix seconds>``. A worker that
  cannot get the lock and has a HIGHER revision than the holder stands by: it rewrites this
  record every STANDBY_HEARTBEAT and retries the lock every STANDBY_POLL, for at most
  STANDBY_MAX. A standby of an equal or higher revision already waiting means it is not needed.
- **The holder hands over** at the start of each loop iteration when a live standby (fresh
  heartbeat, process alive) has a higher revision than its own: it releases the lock and returns;
  the standby takes the lock within STANDBY_POLL. An older worker that grabs the lock meanwhile
  (an older ensure() caller) sees the same standby and hands over before its first tick.
- **Who starts the standby.** ``worker.ensure()`` of a newer release (its MCP tools, CLI and hooks
  call it on SchemaNotReady) starts one when the holder is older and nobody stands by. A process
  that cannot do that (a Rust hook) writes the revision-wanted marker; the older holder then
  starts herdr's current install's worker itself (at most every SPAWN_EVERY), which stands by if
  it is newer.

The Rust worker (P5) keeps the same records. Standard library only.
"""

from __future__ import annotations

import os
import time
from pathlib import Path

STANDBY_POLL = 0.1
STANDBY_HEARTBEAT = 1.0
STANDBY_FRESH = 10.0  # a heartbeat older than this: the standby is gone
STANDBY_MAX = 1800.0
SPAWN_EVERY = 60.0  # the holder starts a successor for a revision-wanted marker at most this often


def _holder_path(lock: Path) -> Path:
    return Path(f"{lock}.holder")


def _standby_path(lock: Path) -> Path:
    return Path(f"{lock}.standby")


def _alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return False


def _write(path: Path, text: str) -> None:
    tmp = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        tmp.write_text(text)
        os.replace(tmp, path)
    except OSError:
        try:
            tmp.unlink()
        except OSError:
            pass


def write_holder(lock: Path, revision: int) -> None:
    """This process holds ``lock`` and runs ``revision``."""
    _write(_holder_path(lock), f"{revision} {os.getpid()}\n")


def holder_revision(lock: Path) -> int:
    """The revision of the worker holding ``lock``: its record while that process lives, else 1 (a
    worker from before the record)."""
    try:
        revision, pid = _holder_path(lock).read_text().split()
        if _alive(int(pid)):
            return int(revision)
    except (OSError, ValueError):
        pass
    return 1


def live_standby(lock: Path, now: float | None = None) -> tuple[int, int] | None:
    """(revision, pid) of a worker standing by for ``lock``, or None."""
    try:
        revision, pid, beat = _standby_path(lock).read_text().split()
        revision, pid, beat = int(revision), int(pid), float(beat)
    except (OSError, ValueError):
        return None
    age = (time.time() if now is None else now) - beat
    if age > STANDBY_FRESH or age < -STANDBY_FRESH or not _alive(pid):
        return None
    return revision, pid


def wait_as_standby(lock: Path, revision: int, take, clock=time.monotonic, sleep=time.sleep):
    """Stand by for ``lock`` as a worker of ``revision``: ``take()`` returns the lock or None.
    Returns the lock once taken, or None (STANDBY_MAX passed, or another standby of an equal or
    higher revision is waiting)."""
    deadline = clock() + STANDBY_MAX
    last_beat = None
    path = _standby_path(lock)
    try:
        while clock() < deadline:
            held = take()
            if held is not None:
                return held
            other = live_standby(lock)
            if other is not None and other[1] != os.getpid() and other[0] >= revision:
                return None  # one standby is enough
            if last_beat is None or clock() - last_beat >= STANDBY_HEARTBEAT:
                _write(path, f"{revision} {os.getpid()} {time.time():.3f}\n")
                last_beat = clock()
            sleep(STANDBY_POLL)
        return None
    finally:
        try:
            if path.read_text().split()[1] == str(os.getpid()):
                path.unlink()
        except (OSError, IndexError):
            pass
