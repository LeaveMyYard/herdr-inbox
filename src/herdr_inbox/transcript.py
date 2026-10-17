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
from datetime import datetime
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
    sid = session_id_or_none(session_id)
    if sid is None:
        return None  # never a glob pattern or a path: only a canonical UUID is looked up
    base = projects_dir()
    hit = _found.get((str(base), sid))
    if hit and hit.is_file():
        _found.move_to_end((str(base), sid))
        return hit  # every herdr agent lookup follows its session (see follow): no glob each time
    for path in base.glob(f"*/{sid}.jsonl"):
        _remember(_found, (str(base), sid), path)
        return path
    return None


CACHE_MAX = 256  # both lookup caches are bounded (LRU): a long-lived worker sees many sessions
_found: "OrderedDict[tuple[str, str], Path]" = OrderedDict()


def _remember(cache: OrderedDict, key, value) -> None:
    cache[key] = value
    cache.move_to_end(key)
    while len(cache) > CACHE_MAX:
        cache.popitem(last=False)


# The ONE session-id validator (plan §1.3 row 4, shared with herdr-panorama): a canonical
# 8-4-4-4-12 hex UUID, any case in, lowercase out; anything else is None. A session id reaches
# us from herdr, the env, hook payloads and transcripts, and becomes part of a file path (find's
# glob), so nothing else ("../x", "*", a slash, surrounding whitespace) is ever used.
_UUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")


def session_id_or_none(value) -> str | None:
    return value.lower() if isinstance(value, str) and _UUID.fullmatch(value) else None


# -- Claude Code session continuation ----------------------------------------------------
# The conversation can move to a new session id in the same process: the old transcript then
# ends with {"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW}, followed
# by bookkeeping records (captured on 2.1.301, 2026-10-16); nothing in NEW points back, and
# herdr / the process env keep reporting OLD.
#
# The rule (plan D6, shared with herdr-panorama's scan/session.py): a continued-in record in the
# MAIN transcript (never a subagent's file, never an isSidechain record) naming another valid
# session id marks the session continued. Only a LATER conversation record (``user`` or
# ``assistant`` whose timestamp is greater than the continued-in's) clears it: the old session
# was resumed (``claude --resume``). Bookkeeping (last-prompt, cost-state, mode,
# file-history-snapshot, any type Claude Code adds tomorrow) never does.
FOLLOW_TAIL = 64 * 1024  # the record is near the end: a cold read looks at the tail only
_CONVERSATION = ("user", "assistant")
# path -> (size read, continued-in id, its time, latest record time seen)
_follow_cache: "OrderedDict[str, tuple[int, str | None, float | None, float | None]]" = OrderedDict()


def _ts(value) -> float | None:
    """A record's ``timestamp`` as epoch seconds (panorama's wire.ts): ISO-8601, or a number
    (milliseconds above 1e11), or None."""
    if value is None or isinstance(value, (bool, list, dict)):
        return None
    if isinstance(value, (int, float)):
        return value / 1000.0 if value > 1e11 else float(value)
    try:
        return datetime.fromisoformat(str(value).replace("Z", "+00:00")).timestamp()
    except (ValueError, TypeError, OverflowError):
        try:
            return _ts(int(value))
        except (ValueError, TypeError, OverflowError):
            return None


def continued_in(path: Path) -> str | None:
    """The session ``path``'s conversation continued in, or None (never continued, or resumed
    and used again since: see the rule above).

    Incremental: a growing transcript (every live session) is read only where it grew since
    the last look, never its whole tail again (review #41). The growth is read whole (up to
    MAX_READ), so the mark and its time survive any amount of bookkeeping between looks."""
    if "subagents" in Path(path).parts[-4:-1]:
        # a subagent's own transcript (<session>/subagents/[workflows/<run>/]agent-<id>.jsonl):
        # only the main one moves the conversation; folders above the session never count
        return None
    key, n = str(path), size(path)
    cached = _follow_cache.get(key)
    if cached and cached[0] == n:
        return cached[1]
    if cached and cached[0] < n and n - cached[0] <= MAX_READ:
        start, nxt, at, last = cached
    else:
        start, nxt, at, last = max(0, n - FOLLOW_TAIL), None, None, None
    try:
        with open(path, "rb") as fh:
            fh.seek(start)
            data = fh.read(n - start)
    except OSError:
        return None
    end = data.rfind(b"\n")  # only complete lines; a partial last line is read next time
    if end == -1:
        return cached[1] if cached else None
    own = session_id_or_none(Path(path).stem)
    for raw in data[:end].split(b"\n"):
        if b'"type"' not in raw:
            continue
        try:
            record = json.loads(raw)
        except (ValueError, RecursionError):
            continue
        if not isinstance(record, dict):
            continue
        kind, when = record.get("type"), _ts(record.get("timestamp"))
        if kind == "continued-in":
            target = session_id_or_none(record.get("continuedInSessionId"))
            if target is not None and target != own and record.get("isSidechain") is not True:
                # an undated record takes the latest time read before it (panorama: f.last_ts)
                nxt, at = target, when or last
        elif nxt is not None and kind in _CONVERSATION and (at is None or (when is not None and when > at)):
            nxt = at = None  # resumed: the old conversation went on after the record
        if when:
            last = when
    _remember(_follow_cache, key, (start + end + 1, nxt, at, last))
    return nxt


def continued_at(path: Path) -> float | None:
    """The time of the continued-in record that makes ``path``'s session continued (see
    continued_in), or None: not continued, or the record (and everything before it) undated."""
    if continued_in(path) is None:
        return None
    cached = _follow_cache.get(str(path))
    return cached[2] if cached else None


_targets_cache: "OrderedDict[str, tuple[int, frozenset]]" = OrderedDict()
_TARGET = re.compile(rb'"continuedInSessionId"\s*:\s*"([^"\\]{0,64})"')  # candidates; session_id_or_none decides


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
                    found.update(sid for m in _TARGET.findall(line)
                                 if (sid := session_id_or_none(m.decode("utf-8", "replace"))))
    except OSError:
        return frozenset()
    result = frozenset(found)
    _remember(_targets_cache, key, (n, result))
    return result


def chain(session_id: str | None, known: str | None = None, max_hops: int = 8) -> list[str]:
    """``session_id`` and every session its conversation continued in, in order (cycle-safe)."""
    out: list[str] = []
    current = session_id_or_none(session_id) or session_id  # canonical when it is a session id
    for hop in range(max_hops):
        if not current or current in out:
            break
        out.append(current)
        path = find(current, known if hop == 0 else None)  # ``known`` is the first session's path
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


OWN = object()  # evidence kind: the conversation itself answered (our CLI's output line)
_evidence_cache: "OrderedDict[str, tuple]" = OrderedDict()  # path -> (stat, offset, edge, found, uses)


_ASSIGN = re.compile(r"^([A-Za-z_]\w*)=(\S+)$")
_QID = re.compile(r"^Q?(\d+)$")
_VALUE_FLAGS = {"--project", "--directory", "--python", "--with", "--from", "-p"}


def own_answer_qids(command: str) -> set[int]:
    """The question ids a shell command ANSWERS with the inbox CLI: only an actual
    invocation at the start of a command segment (start, newline, `;`, `&&`, `||`, `|`):
    `[path/]inbox answer [Q]<n>`, `$VAR answer <n>` with VAR set to an inbox path earlier in
    the command, `uv run [flags] inbox answer <n>`, `python[3] -m herdr_inbox.cli answer <n>`.
    A path, a repo name, a grep pattern or a comment body never counts (review #44 r3)."""
    import shlex

    try:  # quote-aware: `;`, `&`, `|` and newlines inside a quoted answer are text, not separators
        lexer = shlex.shlex(command, posix=True, punctuation_chars=";&|\n")
        lexer.whitespace = " \t\r"
        lexer.whitespace_split = True
        tokens_all = list(lexer)
    except ValueError:
        return set()
    segments, current = [], []
    for tok in tokens_all:
        if tok and set(tok) <= set(";&|\n"):
            segments.append(current)
            current = []
        else:
            current.append(tok)
    segments.append(current)
    inbox_vars, out, elsewhere = set(), set(), False
    for tokens in segments:
        tokens = [t for t in (tokens[:1] and [tokens[0].lstrip("\n")]) + tokens[1:] if t]  # "&&\n$B" glues
        # a CLI pointed at another database (a copy being replayed) answers THAT database's
        # question: from such a setting on, nothing counts (final pass #44 L1)
        sets = [_ASSIGN.match(t).group(1) for t in tokens if _ASSIGN.match(t)]
        if any(name in ("HERDR_INBOX_DB", "XDG_STATE_HOME") for name in sets):
            elsewhere = True
        if elsewhere:
            continue
        if len(tokens) == 1 and _ASSIGN.match(tokens[0]):
            name, value = _ASSIGN.match(tokens[0]).groups()
            if value == "inbox" or value.endswith("/inbox"):
                inbox_vars.add(name)
            continue
        while tokens and _ASSIGN.match(tokens[0]):  # leading env assignments
            tokens = tokens[1:]
        if not tokens:
            continue
        head, rest = tokens[0], None
        if head == "inbox" or head.endswith("/inbox"):
            rest = tokens[1:]
        elif head.startswith("$") and head.strip("${}") in inbox_vars:
            rest = tokens[1:]
        elif head == "uv" and len(tokens) > 2 and tokens[1] == "run":
            i, n = 2, len(tokens)
            while i < n and tokens[i].startswith("-"):
                i += 2 if tokens[i] in _VALUE_FLAGS else 1
            if i < n and (tokens[i] == "inbox" or tokens[i].endswith("/inbox")):
                rest = tokens[i + 1:]
        elif head.split("/")[-1] in ("python", "python3") and tokens[1:3] == ["-m", "herdr_inbox.cli"]:
            rest = tokens[3:]
        if rest and len(rest) >= 2 and rest[0] == "answer":
            qm = _QID.match(rest[1])
            if qm:
                out.add(int(qm.group(1)))
    return out


def own_answer_succeeded(output: str, qid: int) -> bool:
    """Our CLI's own success line for ``qid`` starts a line of the output, and it did not
    say the question was not open."""
    lines = [line.strip() for line in output.splitlines()]
    if any(line.startswith(f"Q{qid} is not open") for line in lines):
        return False
    return any(line.startswith((f"Q{qid} answered, ", f"Q{qid} answered by its own conversation"))
               for line in lines)
_INBOX_READS = ("check_answers", "list_my_questions")


def _stat_key(path: Path) -> tuple | None:
    try:
        st = path.stat()
    except OSError:
        return None
    return (st.st_size, st.st_mtime_ns, st.st_ino)


def _tool_uses(record: dict):
    """(id, kind, qids) of the inbox calls this assistant record makes: `own` for a Bash
    command running `inbox answer <n>` (the conversation answers itself), `read` for
    herdr-inbox's check_answers / list_my_questions MCP tools."""
    if record.get("type") != "assistant":
        return
    content = (record.get("message") or {}).get("content")
    if not isinstance(content, list):
        return
    for block in content:
        if not isinstance(block, dict) or block.get("type") != "tool_use" or not isinstance(block.get("id"), str):
            continue
        name = block.get("name") or ""
        inp = block.get("input") if isinstance(block.get("input"), dict) else {}
        if name == "Bash" and isinstance(inp.get("command"), str) and "inbox" in inp["command"]:
            qids = own_answer_qids(inp["command"])
            if qids:
                yield block["id"], "own", qids
        elif "herdr-inbox" in name and name.endswith(_INBOX_READS):
            yield block["id"], "read", set()


def _tool_results(record: dict):
    """(tool_use_id, text) of the tool results this user record carries."""
    if record.get("type") != "user":
        return
    content = (record.get("message") or {}).get("content")
    if not isinstance(content, list):
        return
    for block in content:
        if not isinstance(block, dict) or block.get("type") != "tool_result":
            continue
        inner, use = block.get("content"), block.get("tool_use_id")
        if isinstance(inner, str):
            yield use, inner
        elif isinstance(inner, list):
            for b in inner:
                if isinstance(b, dict) and isinstance(b.get("text"), str):
                    yield use, b["text"]


def _answers_in(value, out: list):
    """{"id": <int>, "answer": "<str>"} objects anywhere in parsed JSON."""
    if isinstance(value, dict):
        if isinstance(value.get("id"), int) and isinstance(value.get("answer"), str):
            out.append((value["id"], value["answer"]))
        for v in value.values():
            _answers_in(v, out)
    elif isinstance(value, list):
        for v in value:
            _answers_in(v, out)


def evidence(path: Path) -> dict:
    """qid -> [(timestamp, answer | OWN)]: what the conversation's OWN inbox calls gave it
    (review #44 rounds 1-2). A tool result counts only through the tool_use it answers:
    - the conversation's Bash `inbox answer <n> ...` and our CLI's `Q<n> answered` line in
      its result: the conversation answered n itself (OWN);
    - herdr-inbox's check_answers / list_my_questions: `{"id": n, "answer": "<exact>"}`.
    Quoted output (a terminal read, a log, an echo) never counts. Indexed incrementally;
    the cache is keyed on size, mtime and inode (a rewrite is re-indexed)."""
    key, stat = str(path), _stat_key(path)
    if stat is None:
        return {}
    cached = _evidence_cache.get(key)
    if cached and cached[0] == stat:
        return cached[3]
    try:
        with open(path, "rb") as fh:
            grow = False
            if cached and cached[0][2] == stat[2] and cached[1] < stat[0]:
                # an append keeps everything up to the indexed offset: check its last bytes
                fh.seek(max(0, cached[1] - len(cached[2])))
                grow = fh.read(len(cached[2])) == cached[2]
            start, uses, found = (cached[1], dict(cached[4]), {k: list(v) for k, v in cached[3].items()}) if grow \
                else (0, {}, {})
            fh.seek(start)
            data = fh.read(stat[0] - start)
            end = data.rfind(b"\n")
            offset = start + end + 1 if end != -1 else start
            fh.seek(max(0, offset - 64))
            edge = fh.read(min(64, offset))
    except OSError:
        return {}
    if end == -1:
        _remember(_evidence_cache, key, (stat, offset, edge, found, uses))
        return found
    for raw in data[:end].split(b"\n"):
        if b'"tool_use"' in raw and (b"inbox" in raw):
            kind = "use"
        elif b'"tool_result"' in raw and raw.count(b"answer"):
            kind = "result"
        else:
            continue
        try:
            record = json.loads(raw)
        except ValueError:
            continue
        if not isinstance(record, dict):
            continue
        if kind == "use":
            for use_id, what, qids in _tool_uses(record):
                uses[use_id] = (what, frozenset(qids))
            continue
        ts = record.get("timestamp") if isinstance(record.get("timestamp"), str) else None
        for use_id, text in _tool_results(record):
            what, qids = uses.get(use_id, (None, frozenset()))
            if what == "own":
                for qid in qids:
                    if own_answer_succeeded(text, qid):
                        found.setdefault(qid, []).append((ts, OWN))
            elif what == "read" and '"answer"' in text:
                try:
                    parsed = json.loads(text)
                except ValueError:
                    continue
                pairs: list = []
                _answers_in(parsed, pairs)
                for qid, ans in pairs:
                    found.setdefault(qid, []).append((ts, ans))
    # a hit needs the exact (size, mtime, inode); growth continues from the indexed offset
    _remember(_evidence_cache, key, (stat, offset, edge, found, uses))
    return found


def seen_answer(path: Path, qid: int, question: str, answer: str, since: str | None) -> bool:
    """The conversation was GIVEN this answer by a tool, after the answer time: check_answers'
    JSON with this exact answer value, or our CLI's own "Q7 answered" line (it answered it
    itself). Never agent-written text, never a substring, no window before the answer
    (review #44: a false "seen" loses an answer silently)."""
    want = _squash(answer)
    for ts, value in evidence(path).get(qid, []):
        if not _after(ts, since):
            continue
        if value is OWN or _squash(value) == want:
            return True
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
