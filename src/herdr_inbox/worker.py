"""The delivery worker: one herdr-side process that retries every pending answer.

Before 0.6.1 each asking session's own MCP server retried deliveries into its own pane. A
session started before an update keeps its old server (old policy, or no verification at
all), so answers to long-lived sessions could sit undelivered with nobody retrying (live,
2026-10-15: Q49-Q52, 11 minutes). Now:

- exactly one worker runs (an exclusive flock on ``worker.lock`` next to the database);
  ``ensure()`` starts it detached when it is not running, from herdr's ``[[startup]]``, the
  popup, the CLI and every 0.6.1+ MCP server; it exits after WORKER_IDLE_EXIT seconds
  with nothing pending, no enabled remote and no agent in herdr (0.8: it watches agents'
  blocked state, so with agents running it does not idle out);
- every TICK it re-checks unverified acknowledgements (``Store.reconcile``: an old
  server's "typed = seen" is un-seen and, if recent, requeued) and runs the delivery
  policy (service.deliver_batch: idle/done, busy grace, blocked, panel focus, transcript
  verification) for ALL panes; claims serialize it with any old server still retrying;
- answers stuck for NOTIFY_AFTER seconds (or out of tries) raise one herdr notification
  per pane: they can still be fetched with check_answers.
"""

from __future__ import annotations

import fcntl
import logging
import os
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone

from . import herdr, service, tabs
from .store import AGENT_GONE, Question, Store, db_path

log = logging.getLogger("herdr_inbox")

TICK = float(os.environ.get("HERDR_INBOX_WORKER_TICK", "5"))
watching = 0  # agents herdr reported on the last tick: their blocked state is watched (alerts)
IDLE_EXIT = float(os.environ.get("HERDR_INBOX_WORKER_IDLE_EXIT", "600"))
NOTIFY_AFTER = 300.0
LEGACY_RECHECK = timedelta(seconds=60)


def _lock_path(server: str | None = None):
    """One worker per herdr server: the lock is keyed by its socket."""
    import hashlib

    key = hashlib.sha1((server or herdr.server_id()).encode()).hexdigest()[:12]
    return db_path().parent / f"worker-{key}.lock"


def _spawn_env(server: str | None = None) -> dict[str, str]:
    """The spawning process's environment without its venv selection (an MCP server's
    UV_PROJECT_ENVIRONMENT must never make uv resync that venv for the herdr checkout)."""
    env = {k: v for k, v in os.environ.items() if not k.startswith("UV_") and k != "VIRTUAL_ENV"}
    if server:
        env["HERDR_SOCKET_PATH"] = server
    return env


def hold_lock():
    """The worker's lock, or None if another worker holds it. Close the file to release."""
    path = _lock_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    fh = open(path, "a")
    try:
        fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        fh.close()
        return None
    return fh


def is_running(server: str | None = None) -> bool:
    try:
        fh = open(_lock_path(server), "a")
    except OSError:
        return False
    try:
        fcntl.flock(fh, fcntl.LOCK_SH | fcntl.LOCK_NB)
        return False
    except OSError:
        return True
    finally:
        fh.close()


def ensure(server: str | None = None) -> bool:
    """Start the worker for ``server`` (default: this process's herdr server), detached,
    unless one is running. True if one was started. HERDR_INBOX_NO_WORKER=1 turns this off
    (the test suite)."""
    if os.environ.get("HERDR_INBOX_NO_WORKER") or is_running(server):
        return False
    log_path = db_path().parent / "worker.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "a") as err:
        subprocess.Popen([sys.executable, "-m", "herdr_inbox.cli", "worker"],
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=err,
                         start_new_session=True, close_fds=True, env=_spawn_env(server))
    return True


def _ids(qids: list[int]) -> str:
    qids = sorted(qids)
    if len(qids) > 1 and qids == list(range(qids[0], qids[-1] + 1)):
        return f"Q{qids[0]}–Q{qids[-1]}"
    return ", ".join(f"Q{q}" for q in qids)


def _where(q: Question, lookup: bool = True) -> str:
    label = herdr.tab_label(q.tab_id) if q.tab_id and lookup else None
    if label is not None:
        label = tabs.base_label(label, {tabs.DEFAULT_PREFIX})
    return label or q.tab_label or q.pane_id or "its pane"


def _age(iso: str | None) -> float:
    if not iso:
        return 0.0
    return (datetime.now(timezone.utc) - datetime.fromisoformat(iso)).total_seconds()


def _gone(q: Question) -> bool:
    return (q.delivery_error or "").startswith(AGENT_GONE)


def _blocked(q: Question) -> bool:
    return (q.delivery_error or "").startswith("pending: blocked:") and "panel" not in q.delivery_error


def notify_stuck(store: Store, rows: list[Question]) -> None:
    """One notification per pane (and kind) for answers stuck NOTIFY_AFTER seconds or out of
    tries. An agent waiting at a prompt is pointed at; the rest can use check_answers."""
    stuck: dict[tuple[str, bool], list[Question]] = {}
    for q in rows:
        out_of_tries = (q.delivery_error == service.NOT_VERIFIED
                        and q.delivery_attempts >= service.MAX_UNVERIFIED_ATTEMPTS)
        if (out_of_tries or _gone(q) or _age(q.answered_at) >= NOTIFY_AFTER) and not store.notified(q.id):
            kind = "gone" if _gone(q) else "blocked" if _blocked(q) else "stuck"
            stuck.setdefault((q.pane_id or "-", kind), []).append(q)
    for (_, kind), items in stuck.items():
        ids, where = _ids([q.id for q in items]), _where(items[0], lookup=kind != "gone")
        if kind == "gone":
            herdr.notify(f"{ids}: the agent in {where} is gone",
                         "The answer is kept; a new session can fetch it with check_answers.")
        elif kind == "blocked":
            herdr.notify(f"waiting: {where} is at a prompt ({ids})",
                         "Answer the prompt in that pane; the inbox delivers right after.")
        else:
            herdr.notify(f"{ids} not delivered to {where}",
                         "The agent can fetch them with check_answers; or retry from the inbox.")
        store.mark_notified([q.id for q in items])


CAPS = {service.NO_AGENT: service.MAX_STALE_RETRIES, service.CHANGED: service.MAX_STALE_RETRIES,
        service.NOT_VERIFIED: service.MAX_UNVERIFIED_ATTEMPTS}


def served(store: Store) -> dict[str, list[Question]]:
    """The pending answers THIS worker delivers, by pane: its own herdr server's rows, plus
    rows asked before 0.6.1 (no server recorded) whose pane here holds the asking
    conversation -- never on a pane-id-only match. Other servers' rows are theirs."""
    mine = herdr.server_id()
    by_pane: dict[str, list[Question]] = {}
    for q in store.undelivered_all(CAPS):
        if store.server_of(q.id) not in (None, mine):
            continue  # another herdr server's pane: its own worker, never this socket
        by_pane.setdefault(q.pane_id, []).append(q)
    for pane, items in list(by_pane.items()):
        legacy = [q for q in items if store.server_of(q.id) is None]
        if not legacy:
            continue
        # pre-0.6.1 rows may belong to another herdr server: look at most every
        # LEGACY_RECHECK, and after MAX_STALE_RETRIES misses call the agent gone
        due = [q for q in legacy if store.legacy_check_due(q.id, LEGACY_RECHECK)]
        ok: list[Question] = []
        if due:
            agent = herdr.live_agent(pane)
            for q in due:
                if q.session_id and agent and agent["session_id"] == q.session_id:
                    ok.append(q)
                elif store.legacy_miss(q.id) >= service.MAX_STALE_RETRIES:
                    store.mark_agent_gone(q.id, f"no agent of that conversation in pane {pane}")
        by_pane[pane] = [q for q in items if q not in legacy or q in ok]
        if not by_pane[pane]:
            del by_pane[pane]
    return by_pane


def mark_gone(store: Store) -> None:
    """Answers whose 'no agent' / 'conversation changed' retries ran out: the agent is gone.
    Marked once (then never polled again); notify_stuck tells the operator once."""
    for q in store.gone_candidates((service.NO_AGENT, service.CHANGED), service.MAX_STALE_RETRIES):
        store.mark_agent_gone(q.id, q.delivery_error or "the agent is gone")


def tick(store: Store) -> int:
    """One round: reconcile, deliver what this worker serves, notify the stuck. Returns how
    many of ITS answers are still pending (other servers' rows never keep it alive)."""
    global watching
    store.reconcile()
    try:
        service.reconcile_failed(store)  # a 'failed' answer that did land is verified, seen ones lose the error
        service.archive_gone(store)  # gone for a day: out of the popup (check_answers keeps them)
    except Exception:  # noqa: BLE001
        log.warning("failed-answer reconcile failed", exc_info=True)
    try:
        service.revive_moved(store)  # an answer marked gone whose agent's pane only moved
    except Exception:  # noqa: BLE001
        log.warning("moved-pane scan failed", exc_info=True)
    try:
        service.adopt_continuations(store)  # Claude Code session continuation: follow `continued-in`
    except Exception:  # noqa: BLE001
        log.warning("session continuation scan failed", exc_info=True)
    try:
        service.verify_hook_deliveries(store)  # 0.9.0: answers handed over as hook context
    except Exception:  # noqa: BLE001
        log.warning("hook delivery verification failed", exc_info=True)
    try:
        from . import alerts

        watching = alerts.scan_blocked(store)  # agents stuck at an approval/question prompt (never typed into)
    except Exception:  # noqa: BLE001
        watching = 0
        log.warning("blocked-agent scan failed", exc_info=True)
    store.db.execute("DELETE FROM changes WHERE seq <= (SELECT MAX(seq) FROM changes) - 10000")
    mine = herdr.server_id()
    for items in served(store).values():
        try:
            service.deliver_batch(store, items, retry=True)
        except Exception:  # noqa: BLE001 - one pane's trouble never stops the others
            log.exception("delivery to %s failed", items[0].pane_id)
    mark_gone(store)
    left = [q for q in store.stuck_answers() if store.server_of(q.id) in (None, mine)]
    notify_stuck(store, left)
    return sum(len(items) for items in served(store).values())


def running_version() -> str | None:
    """The version of the code this process runs (its package metadata)."""
    try:
        from importlib.metadata import version

        return version("herdr-inbox")
    except Exception:  # noqa: BLE001
        return None


def installed_version() -> str | None:
    """The version on disk now (pyproject.toml of this checkout, else the package metadata)."""
    root = herdr.plugin_root()
    if root:
        try:
            for line in open(os.path.join(root, "pyproject.toml"), encoding="utf-8"):
                if line.startswith("version = "):
                    return line.split('"')[1]
        except (OSError, IndexError):
            pass
    try:
        from importlib.metadata import version

        return version("herdr-inbox")
    except Exception:  # noqa: BLE001
        return None


def start_successor() -> bool:
    """After an update: start the new code's worker from the checkout (uv, its own .venv).
    False if it cannot (no checkout, no uv): then this worker keeps running -- respawning
    this same interpreter would only loop."""
    root = herdr.plugin_root()
    if not root:
        return False
    try:
        subprocess.Popen(["uv", "run", "--quiet", "--frozen", "--no-dev", "--project", root, "inbox", "worker"],
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         start_new_session=True, close_fds=True, env=_spawn_env())
    except OSError:
        return False
    return True


def _pending_now() -> int:
    with Store(timeout=5) as store:
        return sum(len(items) for items in served(store).values())


def run() -> int:
    box = {"lock": hold_lock()}
    if box["lock"] is None:
        return 0  # another worker runs (for this herdr server)
    try:
        return _loop(box)
    finally:
        if box["lock"] is not None:
            box["lock"].close()


def start_remote_thread() -> None:
    import threading

    from . import remote

    threading.Thread(target=remote.loop, name="inbox-remote", daemon=True).start()


def _loop(box: dict) -> int:
    try:
        from . import venvs

        venvs.claim_current()  # spawned from an MCP venv: keep it from being cleaned up
    except BaseException:  # noqa: BLE001
        pass
    mine = running_version()
    handover_failed = False
    idle_since = time.monotonic()
    remote_started = False
    while True:
        try:
            from . import remote

            remotes = bool(remote.enabled_machines())
        except Exception:  # noqa: BLE001
            remotes = False
        if remotes and not remote_started:
            start_remote_thread()  # pulls/flushes remote machines (docs/REMOTE.md)
            remote_started = True
        try:
            with Store(timeout=5) as store:
                pending = tick(store)
        except Exception:  # noqa: BLE001 - keep the worker alive
            log.exception("worker tick failed")
            pending = 1
        disk = installed_version()
        if mine and disk and disk != mine and not handover_failed:  # updated: hand over
            box["lock"].close()
            box["lock"] = None
            if start_successor():
                return 0
            handover_failed = True  # keep serving with this code; try no more
            box["lock"] = hold_lock()
            if box["lock"] is None:
                return 0  # someone else took over meanwhile
        # an enabled remote machine keeps the worker (and its pulls) alive; so does any agent,
        # whose blocked state it watches (an agent at a prompt cannot ask for help itself)
        if pending or remotes or watching:
            idle_since = time.monotonic()
        elif time.monotonic() - idle_since >= IDLE_EXIT:
            box["lock"].close()
            box["lock"] = None
            # an answer committed after this tick's query found the lock still held and
            # spawned nobody: look once more now that the lock is released
            try:
                if _pending_now():
                    ensure()
            except Exception:  # noqa: BLE001
                pass
            return 0
        time.sleep(TICK)
