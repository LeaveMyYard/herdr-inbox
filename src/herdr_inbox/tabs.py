"""Open-question count in the herdr tab label: ``❓2 Farm Issues``.

herdr has no per-tab tokens, so the count goes into the label itself with
``herdr tab rename``. The operator's own label (the *base*) is never stored:
it is always recovered by stripping our marks from the label herdr reports
right now. So a rename by the operator in the meantime is adopted, and a crash
can at worst leave a stale prefix that the next ``inbox sync-tokens`` removes.

- Every mark ever applied is recorded in the database (``tab_marks``), and
  stripping removes all of them, repeatedly: changing the configured mark, or
  two processes that disagree about it, can never stack prefixes.
- The setting has one source that herdr-side processes (popup, sync-tokens)
  and Claude-spawned MCP servers both see: ``config.toml`` in the herdr plugin
  config dir (``herdr plugin config-dir leavemyyard.herdr-inbox``)::

      tab_prefix = "❓"            # "" turns the prefix off
      prefix_unnamed_tabs = false

- Tabs without a custom label show their position ("6"). Prefixing one would
  turn that number into a fixed custom label, because a rename cannot return a
  tab to automatic naming; such tabs are left alone unless
  ``prefix_unnamed_tabs = true``.
- Callers hold the database write lock around count → ``tab get`` →
  ``tab rename`` (see ``service.update_markers``), so concurrent processes
  cannot rename with a stale count.
"""

from __future__ import annotations

import functools
import logging
import os
import re
import tomllib
from pathlib import Path

from . import herdr
from .store import Store

log = logging.getLogger("herdr_inbox")

PLUGIN_ID = "leavemyyard.herdr-inbox"
DEFAULT_PREFIX = "❓"


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


def known_marks(store: Store, current: str | None = None) -> set[str]:
    marks = {r[0] for r in store.db.execute("SELECT mark FROM tab_marks")}
    return (marks | {DEFAULT_PREFIX, current if current is not None else prefix()}) - {""}


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


def apply(store: Store, tab_id: str, count: int, mark: str, current: str | None = None) -> None:
    """Rename ``tab_id`` to show ``count`` if its label (re-read unless given) differs.

    Call with the database write lock held and ``count`` read under it.
    """
    if current is None:
        current = herdr.tab_label(tab_id)
    if current is None:
        return
    want = desired_label(base_label(current, known_marks(store, mark)), count, mark)
    if want == current:
        return
    if want.startswith(mark) and mark:
        store.db.execute("INSERT OR IGNORE INTO tab_marks (mark) VALUES (?)", (mark,))
    herdr.rename_tab(tab_id, want)


def tab_counts(pane_counts: dict[str, int], pane_tabs: dict[str, str | None]) -> dict[str, int]:
    out: dict[str, int] = {}
    for pane, n in pane_counts.items():
        tab = pane_tabs.get(pane)
        if tab:
            out[tab] = out.get(tab, 0) + n
    return out
