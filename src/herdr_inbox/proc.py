"""Is this process serving a Claude that is nested inside another Claude?

A `claude -p` started from an agent's Bash tool inherits the pane's
HERDR_PANE_ID, so herdr identity alone would file its questions (and deliver
answers) as if it were the pane's agent. The process tree tells them apart:
walking up from us, the nearest Claude is our own; if another Claude
sits further up, we are nested. "Claude" is is_claude (shared with herdr-panorama): a native
install, a versioned binary or node running the package, not only a process named `claude`. Checked on a real nested run (MCP server and
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


def _cmdline(pid: int) -> list[str]:
    """The argv of ``pid`` (herdr-panorama hooks._cmdline), or [] if unknown."""
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as fh:
            return [a.decode("utf-8", "replace") for a in fh.read().split(b"\0") if a]
    except FileNotFoundError:
        pass
    except OSError:
        return []
    try:
        return subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True,
                              timeout=1).stdout.split()
    except (OSError, subprocess.SubprocessError):
        return []


def is_claude(comm: str, argv: list[str]) -> bool:
    """A native install (``claude``, or ``~/.local/share/claude/versions/<v>``) or node
    running the ``@anthropic-ai/claude-code`` package."""
    if comm == "claude":
        return True
    first = argv[0] if argv else ""
    if os.path.basename(first) == "claude" or "/claude/versions/" in first:
        return True
    if os.path.basename(first).startswith("node"):
        return any("@anthropic-ai/claude-code" in a or a.endswith("/claude-code/cli.js") for a in argv[1:3])
    return False


def _claude(pid: int, comm: str) -> bool:
    # comm first: argv is read only for a process not named claude (same answer, fewer reads)
    return comm == "claude" or is_claude(comm, _cmdline(pid))


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
        if _claude(pid, comm):
            claudes += 1
            if claudes == 2:
                return True
        pid = ppid
    return False


def owning_claude() -> int | None:
    """The pid of the nearest Claude (is_claude) above us (the session a hook belongs to), else our
    grandparent (hook -> sh -> agent); None if unknown."""
    pid, first = os.getppid(), None
    for depth in range(MAX_DEPTH):
        if pid <= 1:
            break
        info = _parent(pid)
        if info is None:
            break
        ppid, comm = info
        if _claude(pid, comm):
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
