"""Remove old per-version MCP venvs from ${CLAUDE_PLUGIN_DATA}.

Each plugin version runs its MCP server from ``venv-<version>`` (0.5.3+; the shared ``venv``
before that). The data dir is kept across updates, so old ones pile up. On MCP server
start, a venv is removed only if it is for an OLDER version, has not been touched for an
hour (a session on that version is not just creating it), and no running process's
command line names it (``ps``; a venv's python and its ``inbox`` script carry the venv
path). If processes cannot be listed, nothing is removed.
"""

from __future__ import annotations

import logging
import re
import shutil
import subprocess
import time
from pathlib import Path

log = logging.getLogger("herdr_inbox")

SETTLED_SECONDS = 3600


def _version(text: str) -> tuple[int, ...] | None:
    if not re.fullmatch(r"\d+(\.\d+)*", text):
        return None
    return tuple(int(p) for p in text.split("."))


def process_commands() -> list[str] | None:
    try:
        out = subprocess.run(["ps", "-axo", "command="], capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return None
    return out.stdout.splitlines() if out.returncode == 0 else None


def cleanup(data: Path, current: str, commands: list[str] | None) -> list[Path]:
    """Remove the old, settled, unused venvs under ``data``; returns what was removed."""
    mine = _version(current)
    if mine is None or commands is None or not data.is_dir():
        return []
    removed = []
    for path in sorted(data.iterdir()):
        if not path.is_dir() or path.is_symlink():
            continue
        if path.name == "venv":
            older = True  # the shared venv every version before 0.5.3 used
        elif path.name.startswith("venv-") and (v := _version(path.name[5:])) is not None:
            older = v < mine
        else:
            continue
        if not older or time.time() - path.stat().st_mtime < SETTLED_SECONDS:
            continue
        if any(str(path) + "/" in c or c.endswith(str(path)) for c in commands):
            continue
        try:
            shutil.rmtree(path)
            removed.append(path)
        except OSError as exc:
            log.warning("could not remove old venv %s: %s", path, exc)
    return removed


def cleanup_from_env() -> list[Path]:
    """The MCP server's start-up call: ${CLAUDE_PLUGIN_DATA} and this package's version."""
    import os
    from importlib.metadata import PackageNotFoundError, version

    data = os.environ.get("CLAUDE_PLUGIN_DATA")
    if not data:
        return []
    try:
        current = version("herdr-inbox")
    except PackageNotFoundError:
        return []
    return cleanup(Path(data), current, process_commands())


def start_cleanup() -> None:
    """In the background, never fatal: the server must not wait for (or fail on) it."""
    import threading

    def run() -> None:
        try:
            for path in cleanup_from_env():
                log.info("removed old venv %s", path)
        except Exception as exc:  # noqa: BLE001
            log.warning("old venv cleanup: %s", exc)

    threading.Thread(target=run, name="venv-cleanup", daemon=True).start()
