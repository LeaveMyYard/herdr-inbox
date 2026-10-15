"""Remote herdr machines (docs/REMOTE.md).

This module talks to saved herdr SSH machines (``herdr machine list --json``) over plain
ssh: ``BatchMode=yes`` (keys only, never a password or passphrase prompt) with short
timeouts. ``INBOX_SSH`` overrides the ssh binary (tests).
"""

from __future__ import annotations

import json
import os
import re
import shlex
import subprocess
import sys

from . import herdr

REPO = "LeaveMyYard/herdr-inbox"
PLUGIN_ID = "leavemyyard.herdr-inbox"
SSH_OPTIONS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "-o", "ServerAliveInterval=5",
               "-o", "ServerAliveCountMax=2"]


def machines() -> list[dict]:
    """herdr's enabled saved SSH machines: ``{id, label, target, session, ...}``."""
    out, _ = herdr.run_raw("machine", "list", "--json")
    try:
        data = json.loads(out or "[]")
    except ValueError:
        return []
    if isinstance(data, dict):  # tolerate a {"result": ...} envelope
        data = (data.get("result") or {}).get("machines", [])
    return [m for m in data if isinstance(m, dict) and m.get("enabled")]


def find(selector: str) -> dict | None:
    return next((m for m in machines() if selector in (m.get("label"), m.get("id"))), None)


SAFE_TARGET = re.compile(r"[A-Za-z0-9_.@:\[\]-]{1,253}")
SAFE_REF = re.compile(r"[A-Za-z0-9._/-]{1,200}")
SAFE_PATH = re.compile(r"/[A-Za-z0-9._/+-]{1,500}")
PROBE = ('PATH="$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:$PATH"; '
         'for b in uv herdr claude; do command -v "$b" || echo "missing:$b"; done')


def valid_target(target: str) -> bool:
    """An ssh destination that cannot be read as an option (``-oProxyCommand=...`` would run
    a command on THIS machine) or carry shell syntax."""
    return bool(target) and not target.startswith("-") and bool(SAFE_TARGET.fullmatch(target))


def ssh(target: str, command: str, timeout: float = 120.0) -> subprocess.CompletedProcess:
    """Run a FIXED command string on ``target`` (never data: that goes on stdin)."""
    if not valid_target(target):
        raise ValueError(f"refusing ssh target {target!r}")
    argv = [os.environ.get("INBOX_SSH") or "ssh", *SSH_OPTIONS, "--", target, command]
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        return subprocess.CompletedProcess(argv, 124, exc.stdout or "", f"timed out after {timeout:.0f} s")
    except OSError as exc:
        return subprocess.CompletedProcess(argv, 127, "", str(exc))


def probe(target: str) -> dict[str, str]:
    """Absolute paths of uv, herdr and claude on the remote (a non-interactive ssh gets a
    minimal PATH). Missing or odd-looking paths are left out."""
    result = ssh(target, PROBE, timeout=20)
    paths = {}
    for line in (result.stdout or "").splitlines():
        line = line.strip()
        if SAFE_PATH.fullmatch(line):
            paths[line.rsplit("/", 1)[1]] = line
    return paths


def _steps(bins: dict[str, str], upgrade: bool, ref: str | None) -> list[tuple[str, str, tuple[str, ...]]]:
    """(what, remote command, stderr phrases that still count as success). Every part is a
    validated constant: absolute binaries, the repo, a validated ref."""
    claude, herdr_bin = bins["claude"], bins["herdr"]
    source = f"{REPO}#{ref}" if ref else REPO
    herdr_install = f"{herdr_bin} plugin install -y " + (f"--ref {shlex.quote(ref)} " if ref else "") + REPO
    if upgrade:
        return [("Claude marketplace", f"{claude} plugin marketplace update herdr-inbox", ()),
                ("Claude plugin", f"{claude} plugin update herdr-inbox@herdr-inbox", ()),
                ("herdr plugin", herdr_install, ())]
    return [("Claude marketplace", f"{claude} plugin marketplace add {shlex.quote(source)}", ("already",)),
            ("Claude plugin", f"{claude} plugin install herdr-inbox@herdr-inbox", ("already",)),
            ("herdr plugin", herdr_install, ())]


def install(selector: str, upgrade: bool = False, ref: str | None = None) -> int:
    """``inbox remote install|upgrade``: 0 on success, 1 if a step failed, 2 if no such
    enabled machine. Never restarts remote agents."""
    machine = find(selector)
    if machine is None:
        print(f"no enabled herdr machine {selector!r} (see `herdr machine list`)", file=sys.stderr)
        return 2
    target = machine.get("target") or ""
    if not valid_target(target):
        print(f"refusing machine {selector!r}: its ssh target {target!r} could be read as an option", file=sys.stderr)
        return 2
    if ref is not None and not SAFE_REF.fullmatch(ref):
        print(f"refusing --ref {ref!r}: letters, digits and . _ / - only", file=sys.stderr)
        return 2
    bins = probe(target)
    missing = [b for b in ("uv", "herdr", "claude") if b not in bins]
    if missing:
        print(f"[{machine['label']}] not found on the remote: {', '.join(missing)}", file=sys.stderr)
        return 1
    for what, command, ok_phrases in _steps(bins, upgrade, ref):
        print(f"[{machine['label']}] {what}: {command}")
        result = ssh(target, command)
        text = (result.stdout or "") + (result.stderr or "")
        if result.returncode != 0 and not any(p in text.lower() for p in ok_phrases):
            print(f"[{machine['label']}] {what} failed (exit {result.returncode}):\n{text.strip()}", file=sys.stderr)
            return 1
    result = ssh(target, f"{bins['herdr']} plugin list --json")
    version = None
    try:
        plugins = json.loads(result.stdout or "{}").get("result", {}).get("plugins", [])
        version = next((p.get("version") for p in plugins if p.get("plugin_id") == PLUGIN_ID), None)
    except ValueError:
        pass
    if version is None:
        print(f"[{machine['label']}] the herdr plugin is not listed after the install:\n{result.stderr}",
              file=sys.stderr)
        return 1
    print(f"herdr-inbox {version} {'upgraded' if upgrade else 'installed'} on {machine['label']}. "
          "Restart the Claude sessions there to pick it up.")
    return 0
