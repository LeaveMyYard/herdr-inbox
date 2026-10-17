"""The only place that talks to the herdr CLI.

Every call has a short timeout and never raises: failures are logged and the
caller gets ``None`` (or ``False`` plus an error string for delivery).
"""

from __future__ import annotations

import functools
import json
import logging
import os
import re
import subprocess
from pathlib import Path

from . import trace

log = logging.getLogger("herdr_inbox.herdr")

TIMEOUT = 3.0
TOKEN = "inbox_q"
SOURCE = "herdr-inbox"


MAX_ARG = 4000
_SPACE = re.compile(r"[\t\n\r\v\f\u2028\u2029\x85]")
_CONTROL = re.compile(r"[\x00-\x1f\x7f-\x9f]")


def clean(text: str, limit: int = MAX_ARG) -> str:
    """Make agent/operator text safe to hand to herdr, which may type it into a terminal.

    Line and tab breaks become spaces; every other C0/C1 control (ESC, BEL, CSI, ...)
    and DEL is dropped, so no escape or key sequence survives. Capped at ``limit``.
    """
    text = _CONTROL.sub("", _SPACE.sub(" ", text))
    return text if len(text) <= limit else text[: limit - 1] + "…"


def _bin() -> str:
    return os.environ.get("HERDR_BIN_PATH") or "herdr"


def run_raw(*args: str, timeout: float | None = None) -> tuple[str | None, str | None]:
    """Run ``herdr <args>``; return (stdout, None) or (None, error)."""
    argv = [_bin(), *(clean(a) for a in args)]  # every argument, one choke point
    try:
        with trace.step("herdr " + " ".join(args[:2])):
            proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout or TIMEOUT)
    except (OSError, subprocess.SubprocessError) as exc:
        log.warning("herdr %s failed: %s", args[:2], exc)
        return None, str(exc)
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout).strip() or f"exit {proc.returncode}"
        log.warning("herdr %s failed: %s", args[:2], err)
        return None, err
    return proc.stdout, None


def call(*args: str, timeout: float | None = None) -> tuple[dict | None, str | None]:
    """Run ``herdr <args>``; return (``.result`` dict, None) or (None, error)."""
    out, err = run_raw(*args, timeout=timeout)
    if out is None:
        return None, err
    try:
        data = json.loads(out) if out.strip() else {}
    except json.JSONDecodeError:
        return {}, None  # command succeeded but printed no JSON
    return (data.get("result") or {}) if isinstance(data, dict) else {}, None


def pane_identity(pane_id: str) -> dict:
    """tab_id, tab_label and Claude session_id for a pane (missing keys = None)."""
    out = {"tab_id": None, "tab_label": None, "session_id": None}
    res, _ = call("pane", "get", pane_id)
    pane = (res or {}).get("pane") or {}
    out["exists"] = bool(pane)
    out["tab_id"] = pane.get("tab_id")
    out["session_id"] = (pane.get("agent_session") or {}).get("value")
    if out["tab_id"]:
        out["tab_label"] = tab_label(out["tab_id"])
    return out


def tab_label(tab_id: str) -> str | None:
    res, _ = call("tab", "get", tab_id)
    return ((res or {}).get("tab") or {}).get("label")


def rename_tab(tab_id: str, label: str) -> str | None:
    _, err = call("tab", "rename", tab_id, label)
    return err


def pane_tabs() -> dict[str, str | None] | None:
    """pane_id -> tab_id for every pane, or None if herdr is unreachable."""
    res, _ = call("pane", "list")
    if res is None:
        return None
    return {p["pane_id"]: p.get("tab_id") for p in res.get("panes", []) if p.get("pane_id")}


def focused_pane() -> str | None:
    res, _ = call("pane", "list")
    return next((p["pane_id"] for p in (res or {}).get("panes", []) if p.get("focused")), None)


def invocation_pane() -> str | None:
    """The pane that was focused when herdr invoked this plugin command, if herdr says.

    ``HERDR_PLUGIN_CONTEXT_JSON.focused_pane_id`` is the documented source for actions and
    pane commands alike (in a popup, HERDR_PANE_ID is the popup itself).
    """
    try:
        ctx = json.loads(os.environ.get("HERDR_PLUGIN_CONTEXT_JSON") or "{}")
    except json.JSONDecodeError:
        ctx = {}
    return (ctx.get("focused_pane_id") if isinstance(ctx, dict) else None) or None


def all_tabs() -> list[dict] | None:
    """Every tab ({tab_id, label, ...}) in every workspace, or None."""
    res, _ = call("workspace", "list")
    if res is None:
        return None
    tabs = []
    for ws in res.get("workspaces", []):
        res, _ = call("tab", "list", "--workspace", ws["workspace_id"])
        tabs += (res or {}).get("tabs", [])
    return tabs


def plugin_config_dir(plugin_id: str) -> str | None:
    out, _ = run_raw("plugin", "config-dir", plugin_id)
    return out.strip() or None if out else None


def live_agent(pane_id: str) -> dict | None:
    """The agent currently in ``pane_id`` (agent, session_id, status, focused), or None."""
    res, _ = call("agent", "get", pane_id)
    agent = (res or {}).get("agent")
    if not agent:
        return None
    reported = (agent.get("agent_session") or {}).get("value")
    from .store import live_session

    return {
        "agent": agent.get("agent"),
        # after a Claude Code session continuation herdr keeps reporting the old id: the
        # conversation now lives in the session its transcript's `continued-in` names --
        # unless herdr resumed that old id itself (store.live_session)
        "session_id": live_session(reported, pane=pane_id) if reported else reported,
        "reported_session_id": reported,
        "status": agent.get("agent_status"),
        "focused": bool(agent.get("focused")),
    }


def prompt_agent(pane_id: str, text: str) -> str | None:
    """Submit ``text`` to the agent in ``pane_id`` without waiting. Returns an error or None."""
    _, err = call("agent", "prompt", pane_id, text)
    return err


def focus_agent(pane_id: str) -> str | None:
    _, err = call("agent", "focus", pane_id)
    return err


def panel_focused(pane_id: str) -> bool:
    """Claude Code's background-agents panel / a subagent view has the keyboard: typed text
    does not reach the conversation then (probe 2026-10-15). Read from the FOOTER only --
    the first non-empty line below the input box's last ─── border -- so the phrase in
    ordinary output (a diff, a README) never counts."""
    out, _ = run_raw("pane", "read", pane_id, "--source", "visible", "--lines", "20")
    if not out:
        return False
    lines = out.splitlines()
    borders = [i for i, line in enumerate(lines) if line.strip() and set(line.strip()) == {"─"}]
    if not borders:
        return False  # no input box on screen: not Claude Code's footer
    footer = next((line.strip() for line in lines[borders[-1] + 1:] if line.strip()), "")
    return footer.startswith("↑/↓ to select")


_SGR = re.compile(r"\x1b\[([0-9;]*)m")
_OTHER_ESC = re.compile(r"\x1b\[[0-9;?]*[A-Za-ln-z]|\x1b\][^\x07]*\x07|\r")
PROMPT = "\u276f"  # ❯, Claude Code's input-box prompt
# an input-box border: a run of ─, optionally with a session tag before its last dashes
# (`───…─── node ─`, live 2026-10-16)
_BORDER = re.compile(r"^─{3,}(?: [^─\s][^─]{0,40} ─+)?$")


def _undimmed(line: str) -> str:
    """The characters of an ANSI line that are NOT dim (SGR 2): Claude Code draws its
    placeholder ("Try …") dim, and typed text plain."""
    line = _OTHER_ESC.sub("", line)
    out, dim, pos = [], False, 0
    for m in _SGR.finditer(line):
        if not dim:
            out.append(line[pos:m.start()])
        for code in (m.group(1) or "0").split(";"):
            if code in ("0", ""):
                dim = False
            elif code == "2":
                dim = True
            elif code == "22":
                dim = False
        pos = m.end()
    if not dim:
        out.append(line[pos:])
    return "".join(out)


_PLACEHOLDER = re.compile(r'^Try ".*"$')


def composer_state(pane_id: str) -> str:
    """Claude Code's input box: 'empty', 'draft' (anything else: the operator's unsent text,
    a pasted-text or image chip... typing would append to it and submit both, live bug
    2026-10-16 Q124/Q125), or 'unknown' (no input box recognised: never type then).

    The box is the lines between the last two ─── borders on screen; its first line starts
    with ❯ (captured on Claude Code 2.1.301, tests/fixtures/composer). EMPTY is recognised
    positively (review #37): the ❯ line is blank, or holds exactly the dim placeholder
    `Try "…"` (one non-dim cell allowed: a drawn cursor), and every other box line is blank."""
    out, _ = run_raw("pane", "read", pane_id, "--source", "visible", "--lines", "30", "--format", "ansi")
    if not out:
        return "unknown"
    lines = out.split("\n")
    plain = [_SGR.sub("", _OTHER_ESC.sub("", line)) for line in lines]
    borders = [i for i, line in enumerate(plain) if _BORDER.match(line.strip())]
    if len(borders) < 2 or borders[-1] - borders[-2] < 2:
        return "unknown"
    box = range(borders[-2] + 1, borders[-1])
    first = plain[box[0]].replace("\u00a0", " ").strip()
    if not first.startswith(PROMPT):
        return "unknown"
    if any(plain[i].replace("\u00a0", " ").strip() for i in box[1:]):
        return "draft"  # a second line: wrapped or multi-line text
    content = first[len(PROMPT):].strip()
    if not content:
        return "empty"
    undimmed = _undimmed(lines[box[0]]).replace("\u00a0", " ")
    undimmed = undimmed.split(PROMPT, 1)[1] if PROMPT in undimmed else undimmed
    if _PLACEHOLDER.match(content) and len(undimmed.strip()) <= 1:
        return "empty"  # the dim placeholder (a drawn cursor may light one cell)
    return "draft"


def notify(title: str, body: str) -> None:
    call("notification", "show", title, "--body", body, "--sound", "request")


def set_open_count(pane_id: str, count: int) -> None:
    if count > 0:
        flag = ["--token", f"{TOKEN}=?{count}"]
    else:
        flag = ["--clear-token", TOKEN]
    call("pane", "report-metadata", pane_id, "--source", SOURCE, *flag)


def list_pane_ids() -> list[str] | None:
    panes = pane_tabs()
    return None if panes is None else list(panes)


INBOX_PLUGIN_ID = "leavemyyard.herdr-inbox"


def server_id() -> str:
    """Which herdr server this process talks to (its socket): one delivery worker each."""
    if os.environ.get("HERDR_SOCKET_PATH"):
        return os.environ["HERDR_SOCKET_PATH"]
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "herdr", "herdr.sock")


def _own_checkout() -> str | None:
    """The checkout this code runs from (where herdr-plugin.toml is), if it runs from one."""
    here = Path(__file__).resolve()
    for parent in here.parents:
        if (parent / "herdr-plugin.toml").is_file() and (parent / "src" / "herdr_inbox").is_dir():
            return str(parent)
    return None


def _listed_root() -> str | None:
    """The inbox's plugin_root as this herdr server lists it now (None: not listed, or no answer)."""
    res, _ = call("plugin", "list", "--json")
    for p in (res or {}).get("plugins", []):
        if p.get("plugin_id") == INBOX_PLUGIN_ID and p.get("plugin_root"):
            return p["plugin_root"]
    return None


@functools.cache
def plugin_root() -> str | None:
    """This inbox's own checkout (where herdr-plugin.toml is), never the caller's: `inbox
    open` may run from another plugin's action, whose HERDR_PLUGIN_ROOT/ID are its own."""
    return _own_checkout() or _listed_root()  # else an installed (non-editable) copy


_is_herdrs_install: bool | None = None  # decided once herdr has answered (follows_listed_root)


def _same_path(a: str, b: str) -> bool:
    return os.path.realpath(a) == os.path.realpath(b)


def follows_listed_root() -> bool:
    """Whether this process is herdr's own install of the inbox, so it should follow the
    root herdr lists. True when this code runs from no checkout (an installed copy), or
    when its checkout was the listed root the first time herdr listed one. A dev worktree
    or a Claude-cache copy talking to a server that lists another checkout never follows
    it (no handover to other code, no downgrade). Decided once, on herdr's first answer
    (the worker asks at start); until then: no."""
    global _is_herdrs_install
    own = _own_checkout()
    if own is None:
        return True
    if _is_herdrs_install is None:
        listed = _listed_root()
        if listed is None:
            return False
        _is_herdrs_install = _same_path(listed, own)
    return _is_herdrs_install


def current_plugin_root() -> str | None:
    """Where the inbox is installed NOW, for the worker's update check and its successor.
    herdr 0.9.4+ builds every `plugin update` in a new directory and leaves running
    processes their old one, so herdr's install cannot see an update in its own checkout
    (plugin_root, cached for the process): for it this asks herdr every time, never cached.
    A linked checkout is listed at its own path, so there it is the same directory as
    before. Any other copy (follows_listed_root False), or no answer from herdr: this
    code's own checkout, as before."""
    own = _own_checkout()
    if follows_listed_root():
        return _listed_root() or own
    return own


SERVER_GONE = "server_not_running"


def server_gone() -> bool:
    """True only when herdr says this process's server is not running (error code
    ``server_not_running``). Any other failure (no herdr binary, a timeout, another error)
    is not proof that the server is gone, so it is False."""
    _, err = call("workspace", "list")
    if not err:
        return False
    try:
        data = json.loads(err)
    except (json.JSONDecodeError, TypeError):
        return False
    error = data.get("error") if isinstance(data, dict) else None
    return isinstance(error, dict) and error.get("code") == SERVER_GONE


def open_popup(env: dict[str, str] | None = None) -> str | None:
    """Open the inbox's ``inbox`` popup (the herdr ``open`` action, ``inbox open``)."""
    args = ["plugin", "pane", "open", "--plugin", INBOX_PLUGIN_ID, "--entrypoint", "inbox"]
    root = plugin_root()
    if root:
        args += ["--cwd", root]
    for key, value in (env or {}).items():
        args += ["--env", f"{key}={value}"]
    focused = invocation_pane() or os.environ.get("HERDR_PANE_ID")  # focused when the action ran
    if focused:
        args += ["--env", f"INBOX_FOCUS_PANE={focused}"]
    _, err = call(*args)
    return err
