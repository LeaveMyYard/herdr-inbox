"""The only place that talks to the herdr CLI.

Every call has a short timeout and never raises: failures are logged and the
caller gets ``None`` (or ``False`` plus an error string for delivery).
"""

from __future__ import annotations

import json
import logging
import os
import re
import subprocess

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


def run_raw(*args: str, timeout: float = TIMEOUT) -> tuple[str | None, str | None]:
    """Run ``herdr <args>``; return (stdout, None) or (None, error)."""
    argv = [_bin(), *(clean(a) for a in args)]  # every argument, one choke point
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.SubprocessError) as exc:
        log.warning("herdr %s failed: %s", args[:2], exc)
        return None, str(exc)
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout).strip() or f"exit {proc.returncode}"
        log.warning("herdr %s failed: %s", args[:2], err)
        return None, err
    return proc.stdout, None


def call(*args: str, timeout: float = TIMEOUT) -> tuple[dict | None, str | None]:
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
    return {
        "agent": agent.get("agent"),
        "session_id": (agent.get("agent_session") or {}).get("value"),
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


def open_popup() -> str | None:
    """Open the plugin's ``inbox`` popup (used by the herdr ``open`` action)."""
    plugin_id = os.environ.get("HERDR_PLUGIN_ID", "leavemyyard.herdr-inbox")
    args = ["plugin", "pane", "open", "--plugin", plugin_id, "--entrypoint", "inbox"]
    root = os.environ.get("HERDR_PLUGIN_ROOT")
    if root:
        args += ["--cwd", root]
    _, err = call(*args)
    return err
