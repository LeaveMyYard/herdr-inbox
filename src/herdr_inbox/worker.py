"""The delivery worker: one herdr-side process that retries every pending answer.

Before 0.6.1 each asking session's own MCP server retried deliveries into its own pane. A
session started before an update keeps its old server (old policy, or no verification at
all), so answers to long-lived sessions could sit undelivered with nobody retrying (live,
2026-10-15: Q49-Q52, 11 minutes). Now:

- exactly one worker runs (an exclusive flock on ``worker.lock`` next to the database);
  ``ensure()`` starts it detached when it is not running, from herdr's ``[[startup]]``, the
  popup, the CLI and every 0.6.1+ MCP server; it exits after WORKER_IDLE_EXIT seconds
  with nothing pending (the next answer starts it again);
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
from datetime import datetime, timezone

from . import herdr, service, tabs
from .store import Question, Store, db_path

log = logging.getLogger("herdr_inbox")

TICK = float(os.environ.get("HERDR_INBOX_WORKER_TICK", "5"))
IDLE_EXIT = float(os.environ.get("HERDR_INBOX_WORKER_IDLE_EXIT", "600"))
NOTIFY_AFTER = 300.0


def _lock_path():
    return db_path().parent / "worker.lock"


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


def is_running() -> bool:
    try:
        fh = open(_lock_path(), "a")
    except OSError:
        return False
    try:
        fcntl.flock(fh, fcntl.LOCK_SH | fcntl.LOCK_NB)
        return False
    except OSError:
        return True
    finally:
        fh.close()


def ensure() -> bool:
    """Start the worker, detached, unless one is running. True if one was started.
    HERDR_INBOX_NO_WORKER=1 turns this off (the test suite)."""
    if os.environ.get("HERDR_INBOX_NO_WORKER") or is_running():
        return False
    log_path = db_path().parent / "worker.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "a") as err:
        subprocess.Popen([sys.executable, "-m", "herdr_inbox.cli", "worker"],
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=err,
                         start_new_session=True, close_fds=True)
    return True


def _ids(qids: list[int]) -> str:
    qids = sorted(qids)
    if len(qids) > 1 and qids == list(range(qids[0], qids[-1] + 1)):
        return f"Q{qids[0]}–Q{qids[-1]}"
    return ", ".join(f"Q{q}" for q in qids)


def _where(q: Question) -> str:
    label = herdr.tab_label(q.tab_id) if q.tab_id else None
    if label is not None:
        label = tabs.base_label(label, {tabs.DEFAULT_PREFIX})
    return label or q.tab_label or q.pane_id or "its pane"


def _age(iso: str | None) -> float:
    if not iso:
        return 0.0
    return (datetime.now(timezone.utc) - datetime.fromisoformat(iso)).total_seconds()


def notify_stuck(store: Store, rows: list[Question]) -> None:
    """One notification per pane for answers stuck NOTIFY_AFTER seconds or out of tries."""
    stuck: dict[str, list[Question]] = {}
    for q in rows:
        out_of_tries = (q.delivery_error == service.NOT_VERIFIED
                        and q.delivery_attempts >= service.MAX_UNVERIFIED_ATTEMPTS)
        if (out_of_tries or _age(q.answered_at) >= NOTIFY_AFTER) and not store.notified(q.id):
            stuck.setdefault(q.pane_id or "-", []).append(q)
    for items in stuck.values():
        herdr.notify(f"{_ids([q.id for q in items])} not delivered to {_where(items[0])}",
                     "The agent can fetch them with check_answers; or retry from the inbox.")
        store.mark_notified([q.id for q in items])


def tick(store: Store) -> int:
    """One round: reconcile, deliver what is pending in every pane, notify the stuck.
    Returns how many answers are still pending."""
    store.reconcile()
    caps = {service.NO_AGENT: service.MAX_STALE_RETRIES, service.CHANGED: service.MAX_STALE_RETRIES,
            service.NOT_VERIFIED: service.MAX_UNVERIFIED_ATTEMPTS}
    by_pane: dict[str, list[Question]] = {}
    for q in store.undelivered_all(caps):
        by_pane.setdefault(q.pane_id, []).append(q)
    for items in by_pane.values():
        try:
            service.deliver_batch(store, items, retry=True)
        except Exception:  # noqa: BLE001 - one pane's trouble never stops the others
            log.exception("delivery to %s failed", items[0].pane_id)
    left = store.stuck_answers()
    notify_stuck(store, left)
    return len(store.undelivered_all(caps))


def run() -> int:
    lock = hold_lock()
    if lock is None:
        return 0  # another worker runs
    try:
        from . import venvs

        venvs.claim_current()  # spawned from an MCP venv: keep it from being cleaned up
    except BaseException:  # noqa: BLE001
        pass
    idle_since = time.monotonic()
    try:
        while True:
            try:
                with Store(timeout=5) as store:
                    pending = tick(store)
            except Exception:  # noqa: BLE001 - keep the worker alive
                log.exception("worker tick failed")
                pending = 1
            if pending:
                idle_since = time.monotonic()
            elif time.monotonic() - idle_since >= IDLE_EXIT:
                return 0
            time.sleep(TICK)
    finally:
        lock.close()
