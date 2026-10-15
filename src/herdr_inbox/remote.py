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


_machines_cache: tuple[float, list[dict]] | None = None


def enabled_machines(ttl: float = 60.0) -> list[dict]:
    """machines(), cached for ``ttl`` seconds (the worker asks every tick)."""
    global _machines_cache
    import time

    if _machines_cache and time.monotonic() - _machines_cache[0] < ttl:
        return _machines_cache[1]
    found = machines()
    _machines_cache = (time.monotonic(), found)
    return found


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
SAFE_REF = re.compile(r"[A-Za-z0-9._][A-Za-z0-9._/-]{0,199}")  # never a leading "-"
SAFE_PATH = re.compile(r"/[A-Za-z0-9._/+-]{1,500}")
# POSIX sh syntax, run under /bin/sh whatever the remote user's login shell is (fish, ...)
PROBE = ("exec /bin/sh -c '"
         'PATH="$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:$PATH"; '
         'for b in uv herdr claude; do command -v "$b" || echo "missing:$b"; done'
         "'")


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
    """(what, remote command, outputs that still count as success). Every part is a
    validated constant: absolute binaries, the repo, a validated ref. Claude Code exits 0
    for "already on disk" / "already installed" (checked on moneylust 2026-10-15), so a
    non-zero exit is always a failure."""
    claude, herdr_bin = bins["claude"], bins["herdr"]
    source = f"{REPO}#{ref}" if ref else REPO
    herdr_install = f"{herdr_bin} plugin install -y " + (f"--ref {shlex.quote(ref)} " if ref else "") + REPO
    if upgrade:
        return [("Claude marketplace", f"{claude} plugin marketplace update herdr-inbox", ()),
                ("Claude plugin", f"{claude} plugin update herdr-inbox@herdr-inbox", ()),
                ("herdr plugin", herdr_install, ())]
    return [("Claude marketplace", f"{claude} plugin marketplace add {shlex.quote(source)}", ()),
            ("Claude plugin", f"{claude} plugin install herdr-inbox@herdr-inbox", ()),
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


# -- the operator's side: pull, mirror, outbox (docs/REMOTE.md) -----------------------------

import hashlib as _hashlib
import time as _time
import uuid as _uuid
from datetime import datetime as _dt, timedelta as _td, timezone as _tz

EXPORT_FORMAT = 1
CALL_TIMEOUT = 10.0
MAX_EXPORT_BYTES = 2_000_000
MAX_QUESTIONS = 1000
FULL_EVERY = 60
CAPS = {"question": 4000, "header": 60, "tab_label": 120, "pane_id": 64, "asked_by": 120, "title": 120,
        "answer": 4000, "delivery_error": 300, "status": 20, "created_at": 40, "answered_at": 40,
        "delivered_at": 40, "verified_at": 40, "acknowledged_at": 40, "qkey": 64}
STATUSES = ("open", "answered", "withdrawn", "dismissed")
_HEX = re.compile(r"[0-9a-f]{8,64}")
_CTRL = re.compile(r"[\x00-\x08\x0b-\x1f\x7f-\x9f\u200e\u200f\u202a-\u202e\u2066-\u2069]")  # + bidi controls
_LINESEP = re.compile(r"[\u2028\u2029]")


def settings() -> dict:
    from . import tabs

    s = tabs.settings()
    return {"sync_seconds": float(s.get("remote_sync_seconds", 5) or 5),
            "expiry_hours": float(s.get("remote_answer_expiry_hours", 12) or 12)}


def _now() -> _dt:
    return _dt.now(_tz.utc)


def _iso(t: _dt) -> str:
    return t.isoformat(timespec="seconds")


def _text(value, cap: int) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError("not a string")
    return _CTRL.sub("", _LINESEP.sub(" ", value))[:cap]


def _clean_option(o):
    if isinstance(o, str):
        return {"label": _text(o, 200)}
    if isinstance(o, dict) and isinstance(o.get("label"), str):
        out = {"label": _text(o["label"], 200)}
        for k, cap in (("description", 500), ("preview", 4000)):
            if isinstance(o.get(k), str):
                out[k] = _text(o[k], cap)
        return out
    raise ValueError("bad option")


def clean_row(row) -> dict | None:
    """One exported question, validated and capped (remote output is untrusted); None if it
    does not conform."""
    try:
        if not isinstance(row, dict) or not isinstance(row.get("id"), int) or not isinstance(row.get("group"), int):
            return None
        out = {"id": row["id"], "group": row["group"]}
        for key, cap in CAPS.items():
            out[key] = _text(row.get(key), cap)
        if out["status"] not in STATUSES or not out["qkey"] or not _HEX.fullmatch(out["qkey"]) or not out["question"]:
            return None
        opts = row.get("options") or []
        if not isinstance(opts, list) or len(opts) > 9:
            return None
        out["options"] = [_clean_option(o) for o in opts]
        for key in ("multi_select", "hidden", "agent_gone"):
            out[key] = bool(row.get(key))
        return out
    except ValueError:
        return None


def validate_export(raw: str) -> dict:
    if len(raw) > MAX_EXPORT_BYTES:
        raise ValueError("export too large")
    data = json.loads(raw)
    if not isinstance(data, dict):
        raise ValueError("not an object")
    if data.get("format") != EXPORT_FORMAT:
        raise PermissionError(f"inbox {data.get('version', '?')} speaks export format {data.get('format')!r}")
    if not isinstance(data.get("db_id"), str) or not _HEX.fullmatch(data["db_id"]) or not isinstance(data.get("seq"), int):
        raise ValueError("bad header")
    qs = data.get("questions")
    if not isinstance(qs, list) or len(qs) > MAX_QUESTIONS:
        raise ValueError("bad questions")
    rows = [r for r in (clean_row(q) for q in qs) if r is not None]
    ids = data.get("ids")
    if data.get("full") and not (isinstance(ids, list) and all(isinstance(i, int) for i in ids)):
        raise ValueError("full snapshot without ids")
    gone = data.get("gone") or []
    if not (isinstance(gone, list) and all(isinstance(i, int) for i in gone)):
        raise ValueError("bad gone list")
    return {"db_id": data["db_id"], "seq": data["seq"], "full": bool(data.get("full")),
            "version": _text(str(data.get("version", "?")), 20), "ids": ids or [], "gone": gone[:MAX_QUESTIONS],
            "questions": rows}


def _record(store, machine: dict) -> dict:
    store.db.execute("INSERT INTO remote_machines (machine_id, label, target) VALUES (?, ?, ?)"
                     " ON CONFLICT(machine_id) DO UPDATE SET label = excluded.label, target = excluded.target",
                     (machine["id"], machine.get("label"), machine.get("target")))
    row = store.db.execute("SELECT * FROM remote_machines WHERE machine_id = ?", (machine["id"],)).fetchone()
    return dict(row)


def _paths(store, rec: dict) -> dict | None:
    """uv / herdr / plugin root on the remote: probed once, validated, cached."""
    if rec.get("uv") and rec.get("herdr") and rec.get("plugin_root"):
        return rec
    bins = probe(rec["target"])
    if "uv" not in bins or "herdr" not in bins:
        raise RuntimeError("uv/herdr not found on the remote")
    result = ssh(rec["target"], f"{bins['herdr']} plugin list --json", timeout=CALL_TIMEOUT)
    try:
        plugins = json.loads(result.stdout or "{}").get("result", {}).get("plugins", [])
    except ValueError:
        plugins = []
    root = next((p.get("plugin_root") for p in plugins if p.get("plugin_id") == PLUGIN_ID), None)
    if not isinstance(root, str) or not SAFE_PATH.fullmatch(root):
        raise RuntimeError("herdr-inbox is not installed there (inbox remote install)")
    store.db.execute("UPDATE remote_machines SET uv = ?, herdr = ?, plugin_root = ? WHERE machine_id = ?",
                     (bins["uv"], bins["herdr"], root, rec["machine_id"]))
    return {**rec, "uv": bins["uv"], "herdr": bins["herdr"], "plugin_root": root}


def remote_command(rec: dict, sub: str) -> str:
    """The FIXED command line (validated absolute paths, a constant subcommand); data goes on
    stdin, never in here."""
    assert sub in ("export", "apply")
    for key in ("uv", "herdr", "plugin_root"):
        if not SAFE_PATH.fullmatch(rec.get(key) or ""):
            raise ValueError(f"bad {key}")
    return (f"HERDR_BIN_PATH={rec['herdr']} {rec['uv']} run --quiet --frozen --no-dev "
            f"--project {rec['plugin_root']} inbox {sub}")


def control_options() -> list[str]:
    """ssh connection sharing, with a socket path short enough for the 104/108-byte unix
    limit (``%C`` is a 40-char hash): under $XDG_RUNTIME_DIR, else ~/.ssh; off if even that
    is too long (found on a deep checkout path in the moneylust e2e)."""
    base = os.environ.get("XDG_RUNTIME_DIR") or os.path.join(os.path.expanduser("~"), ".ssh")
    path = os.path.join(base, "hinbox-%C")
    if len(path) - 2 + 40 > 100 or not os.path.isdir(base):
        return ["-o", "ControlMaster=no"]
    return ["-o", "ControlMaster=auto", "-o", f"ControlPath={path}", "-o", "ControlPersist=60"]


def call(rec: dict, sub: str, request: dict) -> subprocess.CompletedProcess:
    if not valid_target(rec.get("target") or ""):
        raise ValueError("refusing that ssh target")
    from .store import db_path

    argv = [os.environ.get("INBOX_SSH") or "ssh", *SSH_OPTIONS, *control_options(),
            "--", rec["target"], remote_command(rec, sub)]
    try:
        return subprocess.run(argv, input=json.dumps(request), capture_output=True, text=True, timeout=CALL_TIMEOUT)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(argv, 124, "", f"timed out after {CALL_TIMEOUT:.0f} s")
    except OSError as exc:
        return subprocess.CompletedProcess(argv, 127, "", str(exc))


def _fail(store, rec: dict, error: str) -> None:
    delay = min(60, 5 * 2 ** min(rec.get("failures") or 0, 4))  # 5, 10, 20, 40, 60 s
    store.db.execute("UPDATE remote_machines SET last_error = ?, last_try_at = ?, backoff_until = ?,"
                     " failures = failures + 1 WHERE machine_id = ?",
                     (error[:300], _iso(_now()), _iso(_now() + _td(seconds=delay)), rec["machine_id"]))


def _notify(title: str, body: str) -> None:
    herdr.notify(herdr.clean(title, 200), herdr.clean(body, 300))


def pull(store, machine: dict) -> bool:
    """One pull of ``machine`` into the mirror; True if it worked."""
    with store.transaction():
        rec = _record(store, machine)
    if rec.get("backoff_until") and rec["backoff_until"] > _iso(_now()):
        return False
    try:
        rec = _paths(store, rec)
        cursor = {"db_id": rec["db_id"], "seq": rec["cursor_seq"]} if rec.get("db_id") else None
        full = cursor is None or (rec.get("pulls") or 0) % FULL_EVERY == 0
        result = call(rec, "export", {"cursor": cursor, "full": full})
        if result.returncode == 2:
            raise PermissionError("its inbox has no `export` (inbox remote upgrade)")
        if result.returncode != 0:
            raise RuntimeError((result.stderr or "").strip()[-300:] or f"ssh exit {result.returncode}")
        data = validate_export(result.stdout)
    except PermissionError as exc:
        _fail(store, rec, f"upgrade needed: {exc}")
        return False
    except Exception as exc:  # noqa: BLE001 - offline, garbled, missing: never fatal here
        _fail(store, rec, str(exc))
        return False
    label, new = rec.get("label") or rec["machine_id"], []
    with store.transaction():
        mid = rec["machine_id"]
        if rec.get("db_id") and rec["db_id"] != data["db_id"]:  # the remote inbox was recreated
            store.db.execute("DELETE FROM remote_questions WHERE machine_id = ?", (mid,))
            failed = store.db.execute("UPDATE remote_outbox SET state = 'failed', result = 'reset'"
                                      " WHERE machine_id = ? AND state = 'queued'", (mid,)).rowcount
            if failed:
                new.append(("reset", f"{label}'s inbox was reset", f"{failed} queued answer(s) were not sent — answer again"))
        for row in data["questions"]:
            seen = store.db.execute("SELECT 1 FROM remote_questions WHERE machine_id = ? AND remote_id = ?",
                                    (mid, row["id"])).fetchone()
            store.db.execute(
                "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at)"
                " VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(machine_id, remote_id) DO UPDATE SET"
                " db_id = excluded.db_id, qkey = excluded.qkey, status = excluded.status, data_json = excluded.data_json",
                (mid, data["db_id"], row["id"], row["qkey"], row["status"], json.dumps(row), _iso(_now())))
            if not seen and row["status"] == "open":
                new.append(("question", f"{label} · Q{row['id']} · {row.get('tab_label') or row.get('pane_id') or ''}",
                            row["question"]))
        for rid in data["gone"]:
            store.db.execute("DELETE FROM remote_questions WHERE machine_id = ? AND remote_id = ?", (mid, rid))
        if data["full"]:
            keep = set(data["ids"]) | {r["id"] for r in data["questions"]}
            for (rid,) in store.db.execute("SELECT remote_id FROM remote_questions WHERE machine_id = ?", (mid,)).fetchall():
                if rid not in keep:
                    store.db.execute("DELETE FROM remote_questions WHERE machine_id = ? AND remote_id = ?", (mid, rid))
        store.db.execute(
            "UPDATE remote_machines SET db_id = ?, cursor_seq = ?, version = ?, pulls = pulls + 1, last_ok_at = ?,"
            " last_error = NULL, last_try_at = ?, backoff_until = NULL, failures = 0 WHERE machine_id = ?",
            (data["db_id"], data["seq"], data["version"], _iso(_now()), _iso(_now()), mid))
    for kind, title, body in new:
        _notify(title, service_first_line(body))
    return True


def service_first_line(text: str) -> str:
    line = text.strip().splitlines()[0] if text.strip() else ""
    return line[:200]


# -- the outbox ------------------------------------------------------------------------------


def enqueue(store, machine_id: str, data: dict, action: str, detail: dict | None = None) -> str:
    """Queue an operator action on a mirrored question (the popup); returns its request id."""
    request_id = _uuid.uuid4().hex
    row = store.db.execute("SELECT db_id, qkey FROM remote_questions WHERE machine_id = ? AND remote_id = ?",
                           (machine_id, data["id"])).fetchone()
    if row is None:
        raise LookupError("that remote question is gone")
    # the identity of the question the operator SAW (carried in data), never whatever the
    # mirror holds under that number now (a reset can put a different question there)
    if row[1] != data.get("qkey") or row[0] != data.get("_db_id"):
        raise LookupError("that question changed — reopen it")
    expires = _now() + _td(hours=settings()["expiry_hours"])
    store.db.execute(
        "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, detail_json, created_at,"
        " expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        (request_id, machine_id, row[0], data["id"], row[1], action, json.dumps(detail) if detail else None,
         _iso(_now()), _iso(expires)))
    return request_id


def flush(store, machine: dict) -> None:
    """Send queued actions for ``machine``: FIFO per question, re-validated against the
    mirror first, expired ones failed, each idempotent by its request id."""
    rec = dict(store.db.execute("SELECT * FROM remote_machines WHERE machine_id = ?", (machine["id"],)).fetchone() or {})
    if not rec or not rec.get("last_ok_at"):
        return
    label = rec.get("label") or rec["machine_id"]
    heads, seen = [], set()
    for row in store.db.execute("SELECT * FROM remote_outbox WHERE machine_id = ? AND state = 'queued'"
                                " ORDER BY created_at, rowid", (rec["machine_id"],)).fetchall():
        if row["remote_id"] in seen:
            continue  # FIFO per question: only its oldest queued action goes now
        seen.add(row["remote_id"])
        heads.append(dict(row))
    for row in heads:
        if row["next_try_at"] and row["next_try_at"] > _iso(_now()):
            continue
        rid, ref = row["request_id"], f"{label}:Q{row['remote_id']}"
        if row["expires_at"] <= _iso(_now()):
            _finish(store, rid, "failed", "expired")
            _notify(f"{ref}: your queued answer expired", f"{label} was not reachable in time; answer again if still needed")
            continue
        mirror = store.db.execute("SELECT db_id, qkey, status FROM remote_questions WHERE machine_id = ? AND remote_id = ?",
                                  (rec["machine_id"], row["remote_id"])).fetchone()
        wanted = {"answer": ("open",), "dismiss": ("open",), "deliver": ("answered",), "hide": STATUSES}[row["action"]]
        if mirror is None or mirror[0] != row["db_id"] or mirror[1] != row["qkey"] or mirror[2] not in wanted:
            reason = "not that question any more" if mirror is None or mirror[1] != row["qkey"] else mirror[2]
            _finish(store, rid, "rejected", reason)
            _notify(f"{ref}: {row['action']} not sent", f"rejected: {reason}")
            continue
        request = {"request_id": rid, "db_id": row["db_id"], "id": row["remote_id"], "qkey": row["qkey"],
                   "action": row["action"], "detail": json.loads(row["detail_json"]) if row["detail_json"] else None}
        try:
            result = call({**rec}, "apply", request)
        except ValueError as exc:
            _finish(store, rid, "failed", str(exc))
            continue
        if result.returncode == 2:
            _finish(store, rid, "failed", f"upgrade {label} (inbox remote upgrade {label})")
            continue
        try:
            reply = json.loads(result.stdout or "")
            outcome = reply.get("outcome")
        except (ValueError, AttributeError):
            outcome = None
        if outcome in ("applied", "already_applied"):
            _finish(store, rid, "done", outcome)
        elif outcome == "rejected":
            reason = _text(str(reply.get("reason") or "rejected"), 200)
            _finish(store, rid, "rejected", reason)
            _notify(f"{ref}: {label} rejected your {row['action']}", reason)
        else:  # transport error / lost reply: retry with the SAME request id
            delay = min(60, 5 * 2 ** min(row["attempts"], 4))
            store.db.execute("UPDATE remote_outbox SET attempts = attempts + 1, next_try_at = ?, result = ?"
                             " WHERE request_id = ?",
                             (_iso(_now() + _td(seconds=delay)), (result.stderr or "")[-200:], rid))


def _finish(store, request_id: str, state: str, result: str) -> None:
    store.db.execute("UPDATE remote_outbox SET state = ?, result = ?, sent_at = ? WHERE request_id = ?",
                     (state, result, _iso(_now()), request_id))


def sync_all(store) -> None:
    for machine in machines():
        try:
            if pull(store, machine):
                flush(store, machine)
        except Exception:  # noqa: BLE001 - one machine never stops the others
            import logging

            logging.getLogger("herdr_inbox").warning("remote %s", machine.get("label"), exc_info=True)


def _sync_one(machine: dict) -> None:
    from .store import Store

    try:
        with Store(timeout=5, maintenance=False) as store:  # its own connection: its own thread
            if pull(store, machine):
                flush(store, machine)
    except Exception:  # noqa: BLE001 - one machine never stops the others
        import logging

        logging.getLogger("herdr_inbox").warning("remote %s", machine.get("label"), exc_info=True)


def sync_all_parallel() -> None:
    """Every enabled machine at once (one thread each, capped), so one hung machine (up to
    10 s per call) never delays the others' pulls."""
    from concurrent.futures import ThreadPoolExecutor

    found = enabled_machines()
    if not found:
        return
    with ThreadPoolExecutor(max_workers=min(8, len(found)), thread_name_prefix="inbox-remote") as pool:
        list(pool.map(_sync_one, found))


def loop(stop=None) -> None:
    """The worker's remote thread: pull and flush every ``remote_sync_seconds``."""
    while stop is None or not stop.is_set():
        try:
            sync_all_parallel()
        except Exception:  # noqa: BLE001
            pass
        _time.sleep(settings()["sync_seconds"])


# -- what the popup reads (the local mirror only; never the network) -------------------------


def group_key(machine_id: str, db_id: str, group: int) -> str:
    return f"{machine_id}|{db_id}|{group}"


def _busy_ids(store, machine_id: str) -> set[int]:
    """Remote questions with an answer/dismiss already queued or done: not answerable again."""
    return {r[0] for r in store.db.execute(
        "SELECT remote_id FROM remote_outbox WHERE machine_id = ? AND action IN ('answer', 'dismiss')"
        " AND state IN ('queued', 'done')", (machine_id,))}


def open_groups(store) -> list[tuple[str, dict, list[dict]]]:
    """(entry key, machine record, open questions' data) per open remote group."""
    out = []
    for m in store.db.execute("SELECT * FROM remote_machines ORDER BY label").fetchall():
        m = dict(m)
        busy = _busy_ids(store, m["machine_id"])
        groups: dict[tuple, list[dict]] = {}
        for r in store.db.execute("SELECT db_id, data_json FROM remote_questions WHERE machine_id = ? AND status = 'open'"
                                  " ORDER BY remote_id", (m["machine_id"],)).fetchall():
            data = json.loads(r[1])
            data["_db_id"] = r[0]  # the identity travels with what the popup shows
            if data["id"] in busy:
                continue
            groups.setdefault((r[0], data["group"]), []).append(data)
        for (dbid, group), qs in groups.items():
            out.append((group_key(m["machine_id"], dbid, group), m, qs))
    return out


def group_by_key(store, key: str) -> tuple[dict, list[dict]] | None:
    for k, m, qs in open_groups(store):
        if k == key:
            return m, qs
    return None


def outbox_rows(store) -> list[dict]:
    """Queued actions (⏳) and ones that failed or were rejected (⚠), until seen."""
    rows = store.db.execute(
        "SELECT o.*, m.label FROM remote_outbox o LEFT JOIN remote_machines m USING (machine_id)"
        " WHERE o.state IN ('queued', 'rejected', 'failed') ORDER BY o.created_at").fetchall()
    return [dict(r) for r in rows]


def mark_seen(store, request_id: str) -> None:
    store.db.execute("UPDATE remote_outbox SET state = 'seen' WHERE request_id = ? AND state IN ('rejected', 'failed')",
                     (request_id,))


def offline_note(m: dict) -> str | None:
    """'offline since 10:42 (3 h)' when the last pull failed, else None."""
    if not m.get("last_error"):
        return None
    since = m.get("last_ok_at")
    if not since:
        return "not reached yet"
    when = _dt.fromisoformat(since)
    hours = (_now() - when).total_seconds() / 3600
    age = f"{hours:.0f} h" if hours >= 1 else f"{hours * 60:.0f} min"
    return f"offline since {when.astimezone().strftime('%H:%M')} ({age})"


def as_question(data: dict):
    """A mirrored remote question as a Question (for the popup's views; never stored)."""
    from .store import Question, normalize_options

    return Question(id=data["id"], created_at=data.get("created_at") or "", pane_id=data.get("pane_id"),
                    tab_id=None, tab_label=data.get("tab_label"), session_id=None, cwd=None,
                    asked_by=data.get("asked_by"), question=data["question"],
                    options=normalize_options(data.get("options")) if data.get("options") else None,
                    context=None, norm="", status=data["status"], answer=data.get("answer"),
                    answered_at=data.get("answered_at"), delivered_at=data.get("delivered_at"),
                    delivery_error=data.get("delivery_error"), acknowledged_at=data.get("acknowledged_at"),
                    repeat_count=0, header=data.get("header"), multi_select=bool(data.get("multi_select")),
                    group_id=data.get("group"), group_title=data.get("title"))
