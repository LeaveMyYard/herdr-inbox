"""Verify that a typed answer actually reached the asking Claude conversation.

``herdr agent prompt`` succeeding only means the keys were written to the pane. In live use
(0.4.0) answers typed while the agent was mid-turn left no trace in the conversation. So a
delivery counts only once the session transcript shows it: Claude Code writes a typed prompt
as a ``user`` record (string content) when idle, or as a ``queue-operation`` ``enqueue``
(``content``) and an ``attachment`` ``queued_command`` (``prompt``) when it queues it
mid-turn. Parsing is lenient, like zoetrope's tailer: unknown or broken lines are skipped.
"""

from __future__ import annotations

import json
import os
import re
import time
from pathlib import Path

MAX_READ = 8 * 1024 * 1024  # never read more than this from the tail


def projects_dir() -> Path:
    base = os.environ.get("CLAUDE_CONFIG_DIR") or os.path.expanduser("~/.claude")
    return Path(base) / "projects"


def find(session_id: str | None, known: str | None = None) -> Path | None:
    """The transcript of ``session_id``: the path the SessionStart hook recorded, else a
    lookup by file name under Claude Code's projects directory."""
    if known and Path(known).is_file():
        return Path(known)
    if not session_id or "/" in session_id:
        return None
    for path in projects_dir().glob(f"*/{session_id}.jsonl"):
        return path
    return None


def size(path: Path | None) -> int:
    try:
        return path.stat().st_size if path else 0
    except OSError:
        return 0


def _texts(line: str):
    try:
        record = json.loads(line)
    except (json.JSONDecodeError, ValueError):
        return
    if not isinstance(record, dict):
        return
    kind = record.get("type")
    if kind == "user":
        content = (record.get("message") or {}).get("content")
        if isinstance(content, str):
            yield content
    elif kind == "queue-operation" and record.get("operation") == "enqueue":
        if isinstance(record.get("content"), str):
            yield record["content"]
    elif kind == "attachment":
        attachment = record.get("attachment") or {}
        if attachment.get("type") == "queued_command" and isinstance(attachment.get("prompt"), str):
            yield attachment["prompt"]


def contains(text: str, qids: list[int]) -> bool:
    return text.startswith("[inbox]") and all(re.search(rf"\bQ{q}\b", text) for q in qids)


def landed_since(path: Path, offset: int, qids: list[int]) -> bool:
    """Whether a record appended after ``offset`` carries our message for ``qids``."""
    try:
        with open(path, "rb") as fh:
            end = fh.seek(0, os.SEEK_END)
            fh.seek(max(offset, end - MAX_READ))
            data = fh.read()
    except OSError:
        return False
    for raw in data.split(b"\n"):
        if b"[inbox]" not in raw:
            continue
        for text in _texts(raw.decode("utf-8", errors="replace")):
            if contains(text, qids):
                return True
    return False


def wait_landed(path: Path, offset: int, qids: list[int], timeout: float, interval: float = 0.25) -> bool:
    deadline = time.monotonic() + timeout
    while True:
        if landed_since(path, offset, qids):
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)
