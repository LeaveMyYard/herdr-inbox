"""Keep ``$inbox_q`` visible when herdr-radar owns the sidebar.

herdr-radar (hhdebb/herdr-radar) replaces herdr's agent rows with its own, written
between marker comments in herdr's ``config.toml``::

    # >>> herdr-radar sidebar block
    [ui.sidebar.agents]
    rows = [[...], [..., { token = "$title_unknown", ... }], ["$gap"]]
    [ui.sidebar.agents.rows_by_agent]
    claude = [...]
    ...
    # <<< herdr-radar sidebar block

Those rows have no slot for another plugin's token, so the inbox's ``$inbox_q``
(the ❓ count) never shows. Until radar offers one (hhdebb/herdr-radar#57, an
``extra_tokens`` setting), the operator can run ``inbox radar-sidebar`` (also the
herdr plugin action ``radar-sidebar``), which adds our cell to every agent row that
holds radar's ``$title_*`` tokens, right after ``$title_unknown``. It runs only on
demand -- never from sync-tokens, the startup command or the popup:

- only inside radar's sidebar markers, in ``rows`` of ``[ui.sidebar.agents]`` and
  every entry of ``[ui.sidebar.agents.rows_by_agent]``; everything else in the file
  is kept byte for byte;
- idempotent; never on a row that would exceed herdr's 16-token limit (radar#40:
  one bad row makes herdr reject the whole file);
- the file must parse before and after, or it is left alone; written atomically
  with a backup (``config.toml.inbox-bak``), and only if nobody changed it in the
  meantime; ``herdr server reload-config`` runs only after a change;
- steps aside entirely (adds and removes nothing) once radar's config mentions
  ``extra_tokens`` with ``$inbox_q``: radar then carries the token itself;
- ``--remove`` takes our cells out again (before uninstalling the inbox).

Radar rewrites its block on its configure action and some setting changes, which
drops our cells. The popup then shows a one-line hint (read-only check, off with
``radar_sidebar_hint = false`` in the inbox config.toml).
"""

from __future__ import annotations

import logging
import os
import re
import tomllib
from pathlib import Path

from . import herdr, tabs

log = logging.getLogger("herdr_inbox")

RADAR_ID = "hhdebb.herdr-radar"
START = "# >>> herdr-radar sidebar block"
END = "# <<< herdr-radar sidebar block"
TOKEN = "$inbox_q"
ROW_TOKEN_LIMIT = 16
FALLBACK_FG = "#c04a4a"  # radar's "blocked" red; normally taken from the row's $title_blocked
AGENT_TABLES = ("ui.sidebar.agents", "ui.sidebar.agents.rows_by_agent")

_OUR_CELL = re.compile(r',\s*\{\s*token\s*=\s*"\$inbox_q"[^{}]*\}')


def config_path() -> Path:
    """herdr's config.toml (as radar finds it); HERDR_INBOX_HERDR_CONFIG overrides (tests)."""
    if os.environ.get("HERDR_INBOX_HERDR_CONFIG"):
        return Path(os.environ["HERDR_INBOX_HERDR_CONFIG"])
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return Path(base) / "herdr" / "config.toml"


def hint_enabled() -> bool:
    return bool(tabs.settings().get("radar_sidebar_hint", True))


def _neutral(text: str) -> str:
    """``text`` with string contents and comments blanked (same length), so brackets in a
    ``contains = "["`` rule or a comment are not mistaken for structure."""
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        ch = text[i]
        if ch == "#":
            while i < n and text[i] != "\n":
                out[i] = " "
                i += 1
            continue
        if ch in "\"'":
            quote, i = ch, i + 1
            while i < n and text[i] != quote and text[i] != "\n":
                if quote == '"' and text[i] == "\\":
                    out[i] = " "
                    i += 1
                out[i] = " "
                i += 1
        i += 1
    return "".join(out)


def _close(neutral: str, open_at: int) -> int:
    depth = 0
    for at in range(open_at, len(neutral)):
        if neutral[at] == "[":
            depth += 1
        elif neutral[at] == "]":
            depth -= 1
            if depth == 0:
                return at
    return -1


def _top_rows(neutral: str, open_at: int, close_at: int) -> list[tuple[int, int]]:
    rows, depth, start = [], 0, 0
    for at in range(open_at + 1, close_at):
        if neutral[at] == "[":
            if depth == 0:
                start = at
            depth += 1
        elif neutral[at] == "]":
            depth -= 1
            if depth == 0:
                rows.append((start, at + 1))
    return rows


def tokens_in(row: str) -> int:
    """Tokens a row names, counted like radar does: styled ``token = "$x"`` cells plus bare
    ``"$x"`` cells (a rule's ``contains``/``equals`` value is not a token)."""
    return len(re.findall(r'token\s*=\s*"', row)) + len(re.findall(r'(?<![:=] )"\$\w+"', row))


def _cell(row: str) -> str:
    fg = re.search(r'token\s*=\s*"\$title_blocked"[^{}]*?fg\s*=\s*"(#[0-9a-fA-F]{3,8})"', row)
    return f'{{ token = "{TOKEN}", fg = "{fg.group(1) if fg else FALLBACK_FG}", bold = true, dim = false }}'


def _edit_row(row: str, remove: bool) -> str:
    if remove:
        return _OUR_CELL.sub("", row)
    if '"$title_' not in row or f'"{TOKEN}"' in row or tokens_in(row) + 1 > ROW_TOKEN_LIMIT:
        return row
    neutral = _neutral(row)
    anchor = re.search(r'token\s*=\s*"\$title_unknown"', row) or list(re.finditer(r'token\s*=\s*"\$title_\w+"', row))[-1]
    # the end of the cell (the `{ ... }`) holding the anchor
    depth, at = 0, anchor.end()
    while at < len(row):
        if neutral[at] == "{":
            depth += 1
        elif neutral[at] == "}":
            if depth == 0:
                break
            depth -= 1
        at += 1
    if at >= len(row):
        return row
    return row[: at + 1] + ", " + _cell(row) + row[at + 1 :]


def _edit_block(block: str, remove: bool) -> str:
    """Our cell added to (or removed from) every agent row in radar's sidebar block."""
    neutral = _neutral(block)
    edits: list[tuple[int, int, str]] = []
    table = None
    for line in re.finditer(r"^(?:\[([A-Za-z0-9_.\-]+)\][ \t]*$|[ \t]*([A-Za-z0-9_\-]+)[ \t]*=[ \t]*\[)", neutral, re.M):
        if line.group(1) is not None:
            table = line.group(1).strip()
            continue
        key = line.group(2)
        if table not in AGENT_TABLES or (table == "ui.sidebar.agents" and key != "rows"):
            continue
        open_at = line.end() - 1
        close_at = _close(neutral, open_at)
        if close_at < 0:
            return block
        for start, end in _top_rows(neutral, open_at, close_at):
            new = _edit_row(block[start:end], remove)
            if new != block[start:end]:
                edits.append((start, end, new))
    for start, end, new in reversed(edits):
        block = block[:start] + new + block[end:]
    return block


def stepped_aside(text: str) -> bool:
    """Radar carries ``$inbox_q`` itself (radar#57's ``extra_tokens``): in its block, or
    in radar's own plugin config."""
    sources = [text]
    radar_dir = herdr.plugin_config_dir(RADAR_ID) if "extra_tokens" not in text else None
    if radar_dir:
        try:
            sources.append((Path(radar_dir) / "config.toml").read_text(encoding="utf-8"))
        except OSError:
            pass
    return any(re.search(r"extra_tokens[\s\S]*\$inbox_q", s) for s in sources)


def edited(text: str, remove: bool) -> str | None:
    """The new file text, or None if nothing should change (no block, unparsable, done)."""
    start = text.find(START)
    end = text.find(END, start + 1) if start >= 0 else -1
    if start < 0 or end < 0 or text.count(START) != 1 or text.count(END) != 1:
        return None
    try:
        tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return None  # never touch a file herdr cannot read either
    new = text[:start] + _edit_block(text[start:end], remove) + text[end:]
    if new == text:
        return None
    try:
        tomllib.loads(new)
    except tomllib.TOMLDecodeError:
        log.warning("radar sidebar: edit would break %s; left alone", config_path())
        return None
    return new


def radar_installed() -> bool:
    res, _ = herdr.call("plugin", "list", "--json")
    return any(p.get("plugin_id") == RADAR_ID and p.get("enabled", True) for p in (res or {}).get("plugins", []))


def _write(path: Path, before: str, after: str) -> bool:
    if path.read_text(encoding="utf-8") != before:
        return False  # radar (or the operator) wrote in the meantime: next time
    backup = path.with_name(path.name + ".inbox-bak")
    backup.write_text(before, encoding="utf-8")
    tmp = path.with_name(f".{path.name}.inbox-{os.getpid()}")
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write(after)
        fh.flush()
        os.fsync(fh.fileno())
    os.chmod(tmp, path.stat().st_mode & 0o7777)
    if path.read_text(encoding="utf-8") != before:
        tmp.unlink()
        return False
    os.replace(tmp, path)
    return True


def _read(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None


def missing() -> bool:
    """Read-only: radar is installed and its sidebar rows lack ``$inbox_q`` (for the
    popup's hint). One file read; a herdr call only when the rows lack it."""
    if not hint_enabled():
        return False
    text = _read(config_path())
    if text is None or edited(text, remove=False) is None or stepped_aside(text):
        return False
    return radar_installed()


def ensure(remove: bool = False) -> str:
    """Put ``$inbox_q`` into radar's agent rows (or take it out, ``remove``). Returns what
    happened: ``changed``, ``unchanged``, ``no-config``, ``no-radar``, ``no-block``,
    ``invalid`` (the file does not parse), ``stepped-aside`` or ``conflict`` (the file
    changed meanwhile; run it again)."""
    path = config_path()
    text = _read(path)
    if text is None:
        return "no-config"
    if not radar_installed():
        return "no-radar"
    if text.count(START) != 1 or text.count(END) != 1 or text.find(END) < text.find(START):
        return "no-block"
    try:
        tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return "invalid"
    if stepped_aside(text):
        return "stepped-aside"
    new = edited(text, remove)
    if new is None:
        return "unchanged"
    if not _write(path, text, new):
        return "conflict"
    herdr.run_raw("server", "reload-config")
    return "changed"
