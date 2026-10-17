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

import functools
import os
import subprocess
import threading

MAX_DEPTH = 32
_walk = threading.local()  # without procfs (macOS): the process table, read once per walk


def _ps_table() -> dict[int, tuple[int, str, list[str]]]:
    """pid -> (parent pid, command name, argv) for every process, from two ``ps`` calls: a
    hook walks up to MAX_DEPTH ancestors, and one or two ``ps`` per ancestor cost 10-30 ms each
    on the hottest path (every hook). Kept for one walk only (see _one_walk); {} if ps fails."""
    table = getattr(_walk, "table", None)
    if table is not None:
        return table
    table = {}
    try:
        names = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,comm="], capture_output=True, text=True,
                               timeout=1).stdout
        argvs = subprocess.run(["ps", "-A", "-o", "pid=,command="], capture_output=True, text=True,
                               timeout=1).stdout
        args = {}
        for line in argvs.splitlines():
            parts = line.split()
            if parts and parts[0].isdigit():
                args[int(parts[0])] = parts[1:]
        for line in names.splitlines():
            parts = line.split(None, 2)
            if len(parts) == 3 and parts[0].isdigit() and parts[1].isdigit():
                pid = int(parts[0])
                table[pid] = (int(parts[1]), os.path.basename(parts[2].strip()), args.get(pid, []))
    except (OSError, subprocess.SubprocessError, ValueError):
        table = {}
    if getattr(_walk, "active", False):
        _walk.table = table
    return table


def _one_walk(fn):
    """One walk up the tree sees one process table, and nothing outside it sees a stale one."""
    @functools.wraps(fn)
    def walk():
        _walk.active = True  # the table is None here: every walk clears it on the way out
        try:
            return fn()
        finally:
            _walk.active, _walk.table = False, None
    return walk


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
    row = _ps_table().get(pid)
    return (row[0], row[1]) if row else None


def _cmdline(pid: int) -> list[str]:
    """The argv of ``pid`` (herdr-panorama hooks._cmdline), or [] if unknown."""
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as fh:
            return [a.decode("utf-8", "replace") for a in fh.read().split(b"\0") if a]
    except FileNotFoundError:
        pass  # no procfs (macOS): the walk's ps table (panorama: ps -o command= -p <pid>)
    except OSError:
        return []
    row = _ps_table().get(pid)
    return list(row[2]) if row else []


CLAUDE_ENTRY = ("/@anthropic-ai/claude-code/cli.js", "/@anthropic-ai/claude-code/cli.mjs")


def is_claude(comm: str, argv: list[str]) -> bool:
    """A native install (``claude``, or ``~/.local/share/claude/versions/<v>``) or node
    running the ``@anthropic-ai/claude-code`` entry script (``.../cli.js`` or ``cli.mjs``). A bare
    package name is not Claude: ``npx @anthropic-ai/claude-code`` is the launcher above it."""
    if comm == "claude":
        return True
    first = argv[0] if argv else ""
    if os.path.basename(first) == "claude" or "/claude/versions/" in first:
        return True
    if os.path.basename(first).startswith("node"):
        return any(a.endswith(CLAUDE_ENTRY) for a in argv[1:3])
    return False


def _claude(pid: int, comm: str) -> bool:
    # comm first: argv is read only for a process not named claude (same answer, fewer reads)
    return comm == "claude" or is_claude(comm, _cmdline(pid))


@_one_walk
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


@_one_walk
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
