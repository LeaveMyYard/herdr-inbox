"""Open-question count in the herdr tab label: ``❓2 Farm Issues``.

herdr has no per-tab tokens, so the count goes into the label itself with
``herdr tab rename``. The operator's own label (the *base*) is never stored:
it is always recovered by stripping our prefix from the label herdr reports
right now. So a rename by the operator in the meantime is adopted, not
clobbered, and a crash can at worst leave a stale prefix that the next
``inbox sync-tokens`` removes.

Tabs without a custom label show their position ("6"). Prefixing one would
turn that number into a fixed custom label, because a rename cannot return a
tab to automatic naming; such tabs are left alone unless
``prefix_unnamed_tabs = true``.

Settings, first match wins:
- ``HERDR_INBOX_TAB_PREFIX`` env var (``off`` or empty disables);
- ``config.toml`` in the herdr plugin config dir
  (``herdr plugin config-dir leavemyyard.herdr-inbox``):
  ``tab_prefix = "❓"`` (``""`` disables) and ``prefix_unnamed_tabs = false``.
"""

from __future__ import annotations

import functools
import logging
import os
import re
import tomllib
from pathlib import Path

from . import herdr

log = logging.getLogger("herdr_inbox")

PLUGIN_ID = "leavemyyard.herdr-inbox"
DEFAULT_PREFIX = "❓"


@functools.cache
def _file_settings() -> dict:
    cfg_dir = os.environ.get("HERDR_PLUGIN_CONFIG_DIR") or herdr.plugin_config_dir(PLUGIN_ID)
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
    env = os.environ.get("HERDR_INBOX_TAB_PREFIX")
    if env is not None:
        return "" if env.strip().lower() in ("", "off", "none", "0") else env.strip()
    return str(_file_settings().get("tab_prefix", DEFAULT_PREFIX)).strip()


def prefix_unnamed() -> bool:
    return bool(_file_settings().get("prefix_unnamed_tabs", False))


def base_label(label: str | None) -> str | None:
    """The operator's label: ours stripped (any count), whatever the current prefix setting."""
    if label is None:
        return None
    # every mark we may have applied: default, configured, env (even while disabled now)
    env = (os.environ.get("HERDR_INBOX_TAB_PREFIX") or "").strip()
    marks = {DEFAULT_PREFIX, str(_file_settings().get("tab_prefix", "")).strip(), env} - {"", "off", "none", "0"}
    pattern = "^(?:" + "|".join(re.escape(m) for m in marks) + r")\d+ "
    return re.sub(pattern, "", label, count=1)


def desired_label(base: str, count: int) -> str:
    mark = prefix()
    if count <= 0 or not mark or (base.isdigit() and not prefix_unnamed()):
        return base
    return f"{mark}{count} {base}"


def apply(tab_id: str, count: int) -> None:
    """Re-read the tab's label and rename it only if it differs from what it should be."""
    current = herdr.tab_label(tab_id)
    if current is None:
        return
    want = desired_label(base_label(current), count)
    if want != current:
        herdr.rename_tab(tab_id, want)


def tab_counts(pane_counts: dict[str, int], pane_tabs: dict[str, str | None]) -> dict[str, int]:
    out: dict[str, int] = {}
    for pane, n in pane_counts.items():
        tab = pane_tabs.get(pane)
        if tab:
            out[tab] = out.get(tab, 0) + n
    return out
