"""Open-question count in the herdr tab label: ``❓2 Billing API``.

herdr has no per-tab tokens, so the count goes into the label itself with
``herdr tab rename``. The operator's own label (the *base*) is never stored:
it is always recovered by stripping our marks from the label herdr reports
right now. So a rename by the operator in the meantime is adopted, and a crash
can at worst leave a stale prefix that the next ``inbox sync-tokens`` removes.

- Only tabs the inbox decorated are ever stripped of a custom mark: each tab
  and the marks applied to it are recorded (``tab_decor``) until the tab is
  restored, so an operator label such as ``Q4 roadmap`` is never touched.
  The default mark ``❓`` is stripped anywhere (nobody types it).
- The setting has one source that herdr-side processes (popup, sync-tokens)
  and Claude-spawned MCP servers both see: ``config.toml`` in the herdr plugin
  config dir (``herdr plugin config-dir leavemyyard.herdr-inbox``)::

      tab_prefix = "❓"            # "" turns the prefix off
      prefix_unnamed_tabs = false

- Tabs without a custom label show their position ("6"). Prefixing one would
  turn that number into a fixed custom label, because a rename cannot return a
  tab to automatic naming; such tabs are left alone unless
  ``prefix_unnamed_tabs = true``.
- count → ``tab get`` → ``tab rename`` runs under a cross-process advisory lock
  (``flock`` on a file next to the database), never the SQLite write lock, so a
  slow or wedged herdr cannot block questions, drafts or answers.
"""

from __future__ import annotations

import contextlib
import fcntl
import functools
import logging
import os
import re
import time
import tomllib
from pathlib import Path

from . import herdr
from .store import Store

log = logging.getLogger("herdr_inbox")

PLUGIN_ID = "leavemyyard.herdr-inbox"
DEFAULT_PREFIX = "❓"
LOCK_TIMEOUT = 10.0


@functools.cache
def _config_dir() -> str | None:
    return os.environ.get("HERDR_PLUGIN_CONFIG_DIR") or herdr.plugin_config_dir(PLUGIN_ID)


def settings() -> dict:
    """Read on every call (it is tiny), so an edit applies without restarting anything."""
    cfg_dir = _config_dir()
    if not cfg_dir:
        return {}
    try:
        with open(Path(cfg_dir) / "config.toml", "rb") as fh:
            return tomllib.load(fh)
    except FileNotFoundError:
        return {}
    except (OSError, tomllib.TOMLDecodeError) as exc:
        log.warning("ignoring %s/config.toml: %s", cfg_dir, exc)
        return {}


def prefix() -> str:
    return str(settings().get("tab_prefix", DEFAULT_PREFIX)).strip()


def prefix_unnamed() -> bool:
    return bool(settings().get("prefix_unnamed_tabs", False))


@contextlib.contextmanager
def marker_lock(store: Store, timeout: float = LOCK_TIMEOUT):
    """Serialize label updates across processes; yields False if the lock was not had in time."""
    path = Path(str(store.path) + ".markers.lock")
    with open(path, "a+") as fh:
        deadline = time.monotonic() + timeout
        while True:
            try:
                fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    log.warning("tab labels busy; skipping this update (the next change or sync fixes it)")
                    yield False
                    return
                time.sleep(0.05)
        try:
            yield True
        finally:
            fcntl.flock(fh, fcntl.LOCK_UN)


def marks_for(store: Store, tab_id: str | None) -> set[str]:
    """Marks that may be stripped from this tab: the default, plus any we applied to it."""
    marks = {DEFAULT_PREFIX}
    if tab_id:
        marks |= {r[0] for r in store.db.execute("SELECT mark FROM tab_decor WHERE tab_id = ?", (tab_id,))}
    return marks - {""}


def base_label(label: str | None, marks: set[str]) -> str | None:
    """The operator's label: every leading ``<mark><digits> `` of ours stripped."""
    if label is None or not marks:
        return label
    pattern = re.compile("^(?:" + "|".join(re.escape(m) for m in sorted(marks)) + r")\d+ ")
    while (stripped := pattern.sub("", label, count=1)) != label:
        label = stripped
    return label


def desired_label(base: str, count: int, mark: str) -> str:
    if count <= 0 or not mark or (base.isdigit() and not prefix_unnamed()):
        return base
    return f"{mark}{count} {base}"


def apply(store: Store, tab_id: str, count: int, mark: str) -> None:
    """Re-read ``tab_id``'s label and rename it to show ``count`` if it differs.

    Call inside ``marker_lock`` with ``count`` read under it.
    """
    current = herdr.tab_label(tab_id)
    if current is None:
        return
    base = base_label(current, marks_for(store, tab_id))
    want = desired_label(base, count, mark)
    if want != current:
        if want != base:  # decorating: remember it before the label changes
            store.db.execute("INSERT OR IGNORE INTO tab_decor (tab_id, mark) VALUES (?, ?)", (tab_id, mark))
        if herdr.rename_tab(tab_id, want) is not None:
            return
    if want == base:  # restored (or never needed): stop treating this tab's label as ours
        store.db.execute("DELETE FROM tab_decor WHERE tab_id = ?", (tab_id,))


def tab_counts(pane_counts: dict[str, int], pane_tabs: dict[str, str | None]) -> dict[str, int]:
    out: dict[str, int] = {}
    for pane, n in pane_counts.items():
        tab = pane_tabs.get(pane)
        if tab:
            out[tab] = out.get(tab, 0) + n
    return out
