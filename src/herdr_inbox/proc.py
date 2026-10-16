"""Is this process serving a Claude that is nested inside another Claude?

A `claude -p` started from an agent's Bash tool inherits the pane's
HERDR_PANE_ID, so herdr identity alone would file its questions (and deliver
answers) as if it were the pane's agent. The process tree tells them apart:
walking up from us, the nearest `claude` is our own; if another `claude`
sits further up, we are nested. Checked on a real nested run (MCP server and
hook: claude(child) -> timeout -> zsh -> claude(pane agent) -> zsh -> herdr).

Unknown or unreadable trees count as not nested, so the inbox never switches
itself off by mistake.
"""

from __future__ import annotations

import os
import subprocess

MAX_DEPTH = 32


def _parent(pid: int) -> tuple[int, str] | None:
    """(parent pid, command name) of ``pid``, or None."""
    try:
        with open(f"/proc/{pid}/stat") as fh:
            stat = fh.read()
        comm = stat[stat.index("(") + 1 : stat.rindex(")")]
        return int(stat[stat.rindex(")") + 2 :].split()[1]), comm
    except FileNotFoundError:
        pass  # no procfs (macOS): ask ps
    except (OSError, ValueError, IndexError):
        return None
    try:
        out = subprocess.run(["ps", "-o", "ppid=,comm=", "-p", str(pid)],
                             capture_output=True, text=True, timeout=1).stdout.split(None, 1)
        return int(out[0]), os.path.basename(out[1].strip())
    except (OSError, subprocess.SubprocessError, ValueError, IndexError):
        return None


def nested_claude() -> bool:
    claudes = 0
    pid = os.getppid()
    for _ in range(MAX_DEPTH):
        if pid <= 1:
            break
        info = _parent(pid)
        if info is None:
            break
        ppid, comm = info
        if comm == "claude":
            claudes += 1
            if claudes == 2:
                return True
        pid = ppid
    return False


def owning_claude() -> int | None:
    """The pid of the nearest `claude` above us (the session a hook belongs to), else our
    grandparent (hook -> sh -> agent); None if unknown."""
    pid, first = os.getppid(), None
    for depth in range(MAX_DEPTH):
        if pid <= 1:
            break
        info = _parent(pid)
        if info is None:
            break
        ppid, comm = info
        if comm == "claude":
            return pid
        if depth == 0:
            first = ppid
        pid = ppid
    return first if first and first > 1 else None


def alive(pid: int | None) -> bool:
    if not pid:
        return True  # unknown: the other bounds (supersede, max age) still apply
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
