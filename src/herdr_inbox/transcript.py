"""Verify that a typed answer actually reached the asking Claude conversation.

``herdr agent prompt`` succeeding only means the keys were written to the pane. In live use
(0.4.0) answers typed while the agent was mid-turn left no trace in the conversation. So a
delivery counts only once the session transcript shows it: Claude Code writes a typed prompt
as a ``user`` record (string content) when idle, or as a ``queue-operation`` ``enqueue``
(``content``) and an ``attachment`` ``queued_command`` (``prompt``) when it queues it
mid-turn, and a hook's ``additionalContext`` (0.9.0 hook delivery) as an ``attachment``
``hook_additional_context`` (``content``: a list of strings). Parsing is lenient, like zoetrope's tailer: unknown or broken lines are skipped.
"""

from __future__ import annotations

import json
import os
from collections import OrderedDict
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
    if not session_id or not _SESSION_CHARS.match(session_id):
        return None  # never a glob pattern or a path: no `*?[`, slashes, backslashes or dots
    base = projects_dir()
    hit = _found.get((str(base), session_id))
    if hit and hit.is_file():
        _found.move_to_end((str(base), session_id))
        return hit  # every herdr agent lookup follows its session (see follow): no glob each time
    for path in base.glob(f"*/{session_id}.jsonl"):
        _remember(_found, (str(base), session_id), path)
        return path
    return None


CACHE_MAX = 256  # both lookup caches are bounded (LRU): a long-lived worker sees many sessions
_found: "OrderedDict[tuple[str, str], Path]" = OrderedDict()


def _remember(cache: OrderedDict, key, value) -> None:
    cache[key] = value
    cache.move_to_end(key)
    while len(cache) > CACHE_MAX:
        cache.popitem(last=False)
_SESSION_CHARS = re.compile(r"^[A-Za-z0-9_-]{1,128}$")


# -- Claude Code session continuation ----------------------------------------------------
# The conversation can move to a new session id in the same process: the old transcript then
# ends with {"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW}, followed
# only by bookkeeping records (captured on 2.1.301, 2026-10-16); nothing in NEW points back,
# and herdr / the process env keep reporting OLD.
FOLLOW_TAIL = 64 * 1024  # the record is near the end: only bookkeeping may follow it
_BOOKKEEPING = {"last-prompt", "cost-state", "summary", "custom-title", "ai-title", "agent-name", "mode"}
_follow_cache: "OrderedDict[str, tuple[int, str | None]]" = OrderedDict()  # path -> (size read, continued-in id)
# a session id read from a transcript is only ever used as a canonical UUID: it becomes part of
# a file path (find's glob), so anything else ("../x", "*", a slash) is ignored
_UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")


def continued_in(path: Path) -> str | None:
    """The session ``path``'s conversation continued in, or None. Only when nothing but
    bookkeeping follows the record (an old session resumed and used again is not followed).

    Incremental: a growing transcript (every live session) is read only where it grew since
    the last look, never its whole tail again (review #41)."""
    key, n = str(path), size(path)
    cached = _follow_cache.get(key)
    if cached and cached[0] == n:
        return cached[1]
    start, nxt = (cached[0], cached[1]) if cached and cached[0] < n and n - cached[0] <= FOLLOW_TAIL \
        else (max(0, n - FOLLOW_TAIL), None)
    try:
        with open(path, "rb") as fh:
            fh.seek(start)
            data = fh.read(n - start)
    except OSError:
        return None
    end = data.rfind(b"\n")  # only complete lines; a partial last line is read next time
    if end == -1:
        return cached[1] if cached else None
    for raw in data[:end].split(b"\n"):
        if not raw.strip():
            continue
        if b'"type"' not in raw:
            continue
        try:
            record = json.loads(raw)
        except ValueError:
            continue
        if not isinstance(record, dict):
            continue
        if record.get("type") == "continued-in" and isinstance(record.get("continuedInSessionId"), str):
            nxt = record["continuedInSessionId"] if _UUID.match(record["continuedInSessionId"]) else None
        elif nxt and record.get("type") not in _BOOKKEEPING:
            nxt = None  # the old conversation went on after the record
    _remember(_follow_cache, key, (start + end + 1, nxt))
    return nxt


_targets_cache: "OrderedDict[str, tuple[int, frozenset]]" = OrderedDict()
_TARGET = re.compile(rb'"continuedInSessionId"\s*:\s*"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"')


def continued_targets(path: Path) -> frozenset:
    """Every session ``path``'s conversation EVER continued in, wherever the record sits (an
    old session resumed afterwards keeps its record mid-file). For finding where a delivery
    landed, never for ownership. Incremental and cached like continued_in."""
    key, n = str(path), size(path)
    cached = _targets_cache.get(key)
    if cached and cached[0] == n:
        return cached[1]
    start, found = (cached[0], set(cached[1])) if cached and cached[0] < n else (0, set())
    try:
        with open(path, "rb") as fh:
            fh.seek(start)
            for line in fh:  # whole lines: a record never splits across reads
                if b"continued-in" in line:
                    found.update(m.decode() for m in _TARGET.findall(line))
    except OSError:
        return frozenset()
    result = frozenset(found)
    _remember(_targets_cache, key, (n, result))
    return result


def chain(session_id: str | None, known: str | None = None, max_hops: int = 8) -> list[str]:
    """``session_id`` and every session its conversation continued in, in order (cycle-safe)."""
    out: list[str] = []
    current = session_id
    for _ in range(max_hops):
        if not current or current in out:
            break
        out.append(current)
        path = find(current, known if current == session_id else None)
        current = continued_in(path) if path else None
    return out


def follow(session_id: str | None, known: str | None = None, max_hops: int = 8) -> str | None:
    """The session the conversation of ``session_id`` lives in now (itself if it never
    continued). ``known``: the transcript path recorded for ``session_id``, if any."""
    found = chain(session_id, known, max_hops)
    return found[-1] if found else session_id


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


# 0.10.0: an asyncRewake hook's output wakes the session as a task notification that carries
# it after Claude Code's own label (`Stop hook blocking error from command "Stop": `, 2.1.301).
# Found by our own header anywhere inside, never by that wording (#30 L2): each `[inbox] `
# start, up to the end of its paragraph or the reminder.
_REWAKE_END = re.compile(r"\n\s*\n|</system-reminder>|</task-notification>")


def _rewake_texts(content: str):
    start = content.find("[inbox] ")
    while start != -1:
        m = _REWAKE_END.search(content, start)
        yield content[start:m.start() if m else len(content)].rstrip()
        start = content.find("[inbox] ", start + 1)


def _record_texts(record: dict):
    kind = record.get("type")
    if kind == "user":
        content = (record.get("message") or {}).get("content")
        if isinstance(content, list):  # a list of blocks: only its text blocks are the prompt
            content = "\n".join(b.get("text", "") for b in content
                                if isinstance(b, dict) and b.get("type") == "text" and isinstance(b.get("text"), str))
        if isinstance(content, str) and content:
            yield content
            if "<task-notification>" in content:
                yield from _rewake_texts(content)
    elif kind == "queue-operation" and record.get("operation") == "enqueue":
        if isinstance(record.get("content"), str):
            yield record["content"]
    elif kind == "attachment":
        attachment = record.get("attachment") or {}
        if attachment.get("type") == "queued_command" and isinstance(attachment.get("prompt"), str):
            yield attachment["prompt"]
        elif attachment.get("type") == "hook_additional_context":  # 0.9.0: our hook's delivery
            content = attachment.get("content")
            for text in content if isinstance(content, list) else [content]:
                if isinstance(text, str):
                    yield text


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


def _tails(text: str):
    """(is_start, text from there) for the record's start and for every later ``[inbox] ``.

    A delivery typed onto unsent text in the input box lands AFTER that text, in one user
    record (live 2026-10-16, Q124/Q125). A later start counts only with full evidence: the
    whole typed message (``contains``) or the question's exact answer line (``landed_kinds``)
    from there, never our header alone, so a mere mention never counts."""
    text = _PASTE_WRAPPER.sub("", text, count=1)
    yield True, text
    i = text.find("[inbox] ", 1)
    while i != -1:
        yield False, text[i:]
        i = text.find("[inbox] ", i + 1)


def contains(text: str, qids: list[int], full: str | None = None) -> bool:
    """Our message, naming every id in ``qids`` (see header_ids): at the record's start, or
    later in it when the whole message ``full`` follows verbatim (whitespace folded)."""
    for start, tail in _tails(text):
        if start:
            if set(qids) <= header_ids(tail):
                return True
        elif full and _squash(tail).startswith(_squash(full)) and set(qids) <= header_ids(tail):
            return True
    return False


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


def _squash(text: str) -> str:
    """Runs of whitespace as one space, ends trimmed: compares what was typed with what
    Claude Code recorded, whatever either side did to spacing."""
    return " ".join(text.split())


def landed_exact(path: Path, expected: dict[int, tuple[list[str], str | None]]) -> set[int]:
    return set(landed_kinds(path, expected))


def landed_kinds(path: Path, expected: dict[int, tuple[list[str], str | None]]) -> dict[int, set[int]]:
    """Which ids landed, judged by their FULL expected text, not the id alone: ``expected``
    maps an id to (the exact strings our message carries for it, the answer's time). A
    record counts for an id only if it names the id in our header positions, contains one
    of those strings at a segment start, and is not older than the answer (ids restart if
    the database is recreated)."""
    found: dict[int, set[int]] = {}  # id -> indexes (into its expected strings) that matched
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
            for _, tail in _tails(text):  # the record's start, and every later `[inbox] ` (see _tails)
                named = header_ids(tail)
                stripped = _squash(tail)
                for qid, (segments, since) in expected.items():
                    if qid not in named or not _after(ts, since):
                        continue
                    for i, seg in enumerate(map(_squash, segments)):
                        if stripped.startswith(seg) or ("— " + seg) in stripped or (" · " + seg) in stripped:
                            found.setdefault(qid, set()).add(i)
    return found


def _strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for v in value.values():
            yield from _strings(v)
    elif isinstance(value, list):
        for v in value:
            yield from _strings(v)


def seen_answer(path: Path, qid: int, question: str, answer: str, since: str | None) -> bool:
    """The conversation SAW this answer some other way than our delivery: one record (a tool
    result of the agent reading the inbox, a quoted answer...) holds the id ``Q<qid>``, the
    question's opening AND the exact answer, after the answer time. Live 2026-10-17: the
    resumed agent read Q159-Q166 straight from the inbox database and acted on them."""
    needles = (f"Q{qid}".encode(), f" {qid} ".encode())
    head = _squash(question)[:40]
    want = _squash(answer)
    if not head or not want:
        return False
    # a long answer is evidence on its own next to the id (the agent's own `inbox answer Q7
    # "..."` command precedes the recorded answer time by seconds); a short one also needs
    # the question
    strong = len(want) >= 12
    floor = since
    if since and strong:
        try:
            from datetime import datetime, timedelta

            floor = (datetime.fromisoformat(since) - timedelta(seconds=120)).isoformat(timespec="seconds")
        except ValueError:
            floor = since
    # the id as we write it (Q7), or as the CLI takes it (`inbox answer 7 "..."`)
    word = re.compile(rf"(?<![A-Za-z0-9])Q{qid}(?![0-9])|\banswer\s+Q?{qid}\b")
    try:
        with open(path, "rb") as fh:
            for raw in fh:
                if not any(n in raw for n in needles):
                    continue
                try:
                    record = json.loads(raw)
                except ValueError:
                    continue
                if not isinstance(record, dict):
                    continue
                ts = record.get("timestamp") if isinstance(record.get("timestamp"), str) else None
                if not _after(ts, floor):
                    continue
                text = _squash(" ".join(_strings(record)).replace("\\n", " "))
                if word.search(text) and want in text and (strong or head in text):
                    return True
    except OSError:
        return False
    return False


def landed_since(path: Path, offset: int, qids: list[int], text: str | None = None) -> bool:
    """Whether a record appended after ``offset`` carries our message for ``qids`` (with
    ``text``, the exact message typed, also when it follows other text: see _tails)."""
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
        for record_text in _texts(raw.decode("utf-8", errors="replace")):
            if contains(record_text, qids, text):
                return True
    return False


def wait_landed_any(watch: list[tuple[Path, int]], qids: list[int], timeout: float, interval: float = 0.25,
                    text: str | None = None) -> bool:
    """wait_landed over several transcripts (each from its own offset): typed text lands in
    whichever conversation the pane really runs -- after herdr resumed an older session id,
    that is the OLD session's file, not the one its `continued-in` names (live 2026-10-17)."""
    deadline = time.monotonic() + timeout
    while True:
        if any(landed_since(p, off, qids, text) for p, off in watch):
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)


def wait_landed(path: Path, offset: int, qids: list[int], timeout: float, interval: float = 0.25,
                text: str | None = None) -> bool:
    deadline = time.monotonic() + timeout
    while True:
        if landed_since(path, offset, qids, text):
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)
