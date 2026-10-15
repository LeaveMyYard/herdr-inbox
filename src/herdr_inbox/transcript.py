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


_PASTE_WRAPPER = re.compile(r"^\s*(?:<pasted_content[^>]*>\s*)?")


def _texts(line: str):
    for text, _ in _records(line):
        yield text


def _records(line: str):
    """(text, timestamp or None) of the prompt-carrying records on one line."""
    try:
        record = json.loads(line)
    except (json.JSONDecodeError, ValueError):
        return
    if not isinstance(record, dict):
        return
    ts = record.get("timestamp") if isinstance(record.get("timestamp"), str) else None
    for text in _record_texts(record):
        yield text, ts


def _record_texts(record: dict):
    kind = record.get("type")
    if kind == "user":
        content = (record.get("message") or {}).get("content")
        if isinstance(content, str):
            yield content
        elif isinstance(content, list):  # a list of blocks: only its text blocks are the prompt
            text = "\n".join(b.get("text", "") for b in content
                             if isinstance(b, dict) and b.get("type") == "text" and isinstance(b.get("text"), str))
            if text:
                yield text
    elif kind == "queue-operation" and record.get("operation") == "enqueue":
        if isinstance(record.get("content"), str):
            yield record["content"]
    elif kind == "attachment":
        attachment = record.get("attachment") or {}
        if attachment.get("type") == "queued_command" and isinstance(attachment.get("prompt"), str):
            yield attachment["prompt"]


_SINGLE = re.compile(r"\[inbox\] Q(\d+) answered — ")
_BATCH = "[inbox] answered — "
_FIRST_SEGMENT = re.compile(r"Q(\d+) ")
_NEXT_SEGMENT = re.compile(r" · Q(\d+) ")


def header_ids(text: str) -> set[int]:
    """The Q ids our own delivery format names, read only where it writes them:
    ``[inbox] Q7 answered — …`` or ``[inbox] answered — Q7 … · Q8 …``. An id merely
    mentioned in a question or an answer ("same as Q28") does not count. A long prompt is
    recorded wrapped as ``\n\n<pasted_content id="…">\n[inbox] …`` (Claude Code collapses
    big pastes; checked 2026-10-14), so leading whitespace and that wrapper are skipped."""
    text = _PASTE_WRAPPER.sub("", text, count=1)
    if m := _SINGLE.match(text):
        return {int(m.group(1))}
    if not text.startswith(_BATCH):
        return set()
    rest = text[len(_BATCH):]
    ids = {int(m.group(1)) for m in _NEXT_SEGMENT.finditer(rest)}
    if m := _FIRST_SEGMENT.match(rest):
        ids.add(int(m.group(1)))
    return ids


def contains(text: str, qids: list[int]) -> bool:
    """Our message, naming every id in ``qids`` (see header_ids)."""
    return set(qids) <= header_ids(text)


def landed_ids(path: Path, candidates: list[int]) -> set[int]:
    """Which of ``candidates`` appear in an ``[inbox]`` record anywhere in the transcript tail
    (used to re-check deliveries recorded without verification)."""
    found: set[int] = set()
    try:
        with open(path, "rb") as fh:
            end = fh.seek(0, os.SEEK_END)
            fh.seek(max(0, end - MAX_READ))
            data = fh.read()
    except OSError:
        return found
    for raw in data.split(b"\n"):
        if b"[inbox]" not in raw:
            continue
        for text in _texts(raw.decode("utf-8", errors="replace")):
            for qid in candidates:
                if qid not in found and contains(text, [qid]):
                    found.add(qid)
    return found


def _after(ts: str | None, since: str | None) -> bool:
    """The record is not older than ``since`` (lenient: unknown times count as newer)."""
    if not ts or not since:
        return True
    try:
        from datetime import datetime

        return datetime.fromisoformat(ts.replace("Z", "+00:00")) >= datetime.fromisoformat(since)
    except ValueError:
        return True


def landed_exact(path: Path, expected: dict[int, tuple[list[str], str | None]]) -> set[int]:
    """Which ids landed, judged by their FULL expected text, not the id alone: ``expected``
    maps an id to (the exact strings our message carries for it, the answer's time). A
    record counts for an id only if it names the id in our header positions, contains one
    of those strings at a segment start, and is not older than the answer (ids restart if
    the database is recreated)."""
    found: set[int] = set()
    try:
        with open(path, "rb") as fh:
            end = fh.seek(0, os.SEEK_END)
            fh.seek(max(0, end - MAX_READ))
            data = fh.read()
    except OSError:
        return found
    for raw in data.split(b"\n"):
        if b"[inbox]" not in raw:
            continue
        for text, ts in _records(raw.decode("utf-8", errors="replace")):
            stripped = _PASTE_WRAPPER.sub("", text, count=1)
            named = header_ids(stripped)
            for qid, (segments, since) in expected.items():
                if qid in found or qid not in named or not _after(ts, since):
                    continue
                if any(stripped.startswith(seg) or ("— " + seg) in stripped or (" · " + seg) in stripped
                       for seg in segments):
                    found.add(qid)
    return found


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
