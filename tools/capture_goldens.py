"""Capture the inbox's behaviour goldens of the Rust rewrite plan (herdr-panorama
docs/plans/rust-rewrite.md §5.2) from the running Python code. Nothing in them is written by hand.

- ``tests/fixtures/screens/*.json`` (item 2, app-screen goldens, the inbox popup's spec): the
  popup (``InboxApp``) runs headless (``run_test``) through the scenarios below, taken from
  ``tests/make_screenshots.py`` and ``tests/test_tui.py``; every screen is what Textual's
  compositor sends to the terminal, as runs of ``[text, fg, bg, bold]`` per row (the shape of
  panorama's ``design_*.json`` and ``screens/*.json``; ``null`` is the terminal's default colour).
- ``tests/fixtures/schema.json``: the schema this code's migration produces (``store._expected_schema``:
  every table's columns, every index, trigger and view by name). The Rust hooks never migrate; on a
  database not stamped with this revision they check it against this list, as
  ``store.schema_missing`` does.
- ``tests/fixtures/landed.json``: ``transcript.landed_kinds`` over generated transcript records
  (every prompt-carrying record type, single and batch messages, references, pastes, text typed
  onto a draft, mentions, older and undated records) for questions whose expected strings come
  from ``service.expected_segments``. The Rust hooks' landing check must give the same answers.
- ``tests/fixtures/store-questions.json``: question reads and writes of ``store.Store`` (insert,
  groups, drafts, answers, status, repeats, acknowledgement, redelivery, the owner queries) run as
  a script on a fresh migrated database, with every table they touch dumped after each step
  (times as ``<time>``), plus ``store.normalize`` and ``store.clean_title`` over a corpus. The Rust
  store replays the script and must leave the same rows.
- ``tests/fixtures/decisions.json`` (item 5, the delivery decision table ``inbox-core::decide``
  must reproduce): the test suite runs in its own pytest with ``tests/decision_recorder.py``,
  which turns on the test-only trace (``herdr_inbox.decision_trace``) and records every delivery
  decision the tests drive, normalised and deduplicated.

Deterministic: fixed sizes and data, TZ=UTC, every ``HERDR_*`` variable dropped, a temporary
inbox database, the herdr binary the tests' fake (``tests/conftest.py``), the arming delays at
0 (as in the tests), text cursors held on, and the operator's own inbox, config, transcripts and
herdr never read. ``tests/test_goldens.py`` runs this into a temporary directory and compares.
Regenerate after a deliberate behaviour change (and say why in the PR):

    uv run python tools/capture_goldens.py                    # rewrite screens/ and decisions.json
    uv run python tools/capture_goldens.py --only decisions   # the decision table alone (whole suite)
    uv run python tools/capture_goldens.py --out DIR          # write under DIR instead
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "fixtures"
SID = "aaaaaaaa-0000-4000-8000-0000000000%02d"  # session ids: canonical, fixed


def isolate(work: Path) -> None:
    """The process's environment: nothing of the operator's (tests/conftest.py's fake_herdr rules)."""
    for key in [k for k in os.environ if k.startswith("HERDR_") or k.startswith("FAKE_HERDR_")
                or k.startswith("INBOX_") or k.startswith("TEXTUAL")
                or k in ("CLAUDE_CODE_SESSION_ID", "CLAUDE_PLUGIN_DATA", "NO_COLOR", "FORCE_COLOR", "COLORTERM",
                         "TERM", "COLUMNS", "LINES")]:
        del os.environ[key]
    cfg = work / "plugin-config"
    cfg.mkdir(parents=True)
    (cfg / "config.toml").write_text('tab_prefix = ""\n')
    os.environ.update({
        "TZ": "UTC",
        "HERDR_INBOX_DB": str(work / "inbox.db"),
        "HERDR_PANE_ID": "w1:p1",
        "CLAUDE_CONFIG_DIR": str(work / "claude"),
        "HERDR_INBOX_NO_WORKER": "1",
        "HERDR_INBOX_HERDR_CONFIG": str(work / "herdr-config" / "config.toml"),
        "HERDR_PLUGIN_CONFIG_DIR": str(cfg),
        "FAKE_HERDR_CONFIG_DIR": str(cfg),
        "FAKE_HERDR_SESSION": SID % 1,
        "XDG_CONFIG_HOME": str(work / "xdg-config"),
        "XDG_STATE_HOME": str(work / "xdg-state"),
    })
    time.tzset()
    sys.path.insert(0, str(ROOT))


def write(path: Path, doc: dict) -> None:
    """One screen row per line: small diffs when behaviour changes."""
    path.parent.mkdir(parents=True, exist_ok=True)
    parts = []
    for name, rows in doc["screens"].items():
        body = ",\n".join("    " + json.dumps(r, ensure_ascii=False, separators=(",", ":")) for r in rows)
        parts.append(f"  {json.dumps(name)}: [\n{body}\n  ]")
    head = {k: v for k, v in doc.items() if k != "screens"}
    text = json.dumps(head, ensure_ascii=False)[:-1] + ', "screens": {\n' + ",\n".join(parts) + "\n}}\n"
    json.loads(text)  # well-formed
    path.write_text(text)


# -- item 2: the popup's screens ------------------------------------------------------------

def hexcolor(c) -> str | None:
    if c is None or c.is_default:
        return None
    return c.get_truecolor().hex.lower()


def dump(app) -> list[list]:
    """The screen as the terminal gets it (export_screenshot's own render), runs per row.
    ``reverse`` is resolved (fg and bg swapped), the only attribute besides bold the popup
    draws with; dim, italic and underline never reach its output."""
    update = app.screen._compositor.render_update(full=True, screen_stack=app._background_screens)
    rows = []
    for strips in update.strips:
        row: list[list] = []
        for strip in strips:
            for seg in strip:
                if seg.control or not seg.text:
                    continue
                st = seg.style
                fg, bg = (hexcolor(st.color), hexcolor(st.bgcolor)) if st else (None, None)
                if st and st.reverse:  # what the terminal shows: the selected tab, chips, cursors
                    fg, bg = bg, fg
                attrs = (fg, bg, bool(st and st.bold))
                if row and tuple(row[-1][1:]) == attrs:
                    row[-1][0] += seg.text
                else:
                    row.append([seg.text, *attrs])
        rows.append(row)
    return rows


def no_blink(app) -> None:
    """A text input's cursor blinks on a wall-clock timer: held on."""
    from textual.widgets import Input

    for w in app.query(Input):
        w.cursor_blink = False
    for screen in app.screen_stack:
        for w in screen.query(Input):
            w.cursor_blink = False


class Shots:
    def __init__(self):
        self.files: dict[str, dict] = {}

    async def add(self, file: str, source: str, size: tuple[int, int], name: str, app, pilot) -> None:
        await pilot.pause()
        no_blink(app)
        await pilot.pause()
        doc = self.files.setdefault(file, {"source": source, "size": list(size), "screens": {}})
        assert doc["size"] == list(size) and name not in doc["screens"], (file, name)
        doc["screens"][name] = dump(app)


def fresh_store(work: Path, name: str):
    """A new, empty inbox database for one scenario (migrated, as the popup leaves it)."""
    from herdr_inbox.store import Store

    os.environ["HERDR_INBOX_DB"] = str(work / f"{name}.db")
    return Store(migrate=True)


def callers():
    from herdr_inbox.service import Caller

    billing = Caller(pane_id="w1:p1", tab_id="w1:t1", tab_label="Billing API", session_id=SID % 1, cwd="/r")
    search = Caller(pane_id="w1:p2", tab_id="w1:t2", tab_label="Search refactor", session_id=SID % 2, cwd="/r")
    docs = Caller(pane_id="w1:p3", tab_id="w1:t3", tab_label="Docs site", session_id=SID % 3, cwd="/r")
    return billing, search, docs


DB = [{"label": "Postgres", "description": "keeps the current ORM and migrations"},
      {"label": "SQLite", "description": "single file, no server"}, {"label": "DuckDB"}]


# The held instant: every row's age is shown as of this time (see hold_clock).
HELD = "2026-10-17T12:00:00+00:00"
FIRST = 10  # above every id a scenario creates: the newest row is the youngest


def hold(store) -> None:
    """Fixed ``created_at`` for every row, in id order as the real code assigns them: Q<n> was
    asked 7 * (FIRST - n) minutes before HELD, alert A<n> raised 3 * (FIRST - n) hours before it."""
    from datetime import datetime, timedelta

    held = datetime.fromisoformat(HELD)
    for table, step in (("questions", timedelta(minutes=7)), ("alerts", timedelta(hours=3))):
        for (rid,) in store.db.execute(f"SELECT id FROM {table}").fetchall():
            stamp = (held - step * (FIRST - rid)).isoformat(timespec="seconds")
            store.db.execute(f"UPDATE {table} SET created_at = ? WHERE id = ?", (stamp, rid))


def hold_clock() -> None:
    """The popup shows ages (``tui._age``, cli._age's formatting) as of HELD: the real code
    formats them, only the instant it measures from is held (shifted by now - HELD)."""
    from datetime import datetime, timezone

    from herdr_inbox import cli, tui

    held = datetime.fromisoformat(HELD)

    def age(iso: str) -> str:
        shift = datetime.now(timezone.utc) - held
        return cli._age((datetime.fromisoformat(iso) + shift).isoformat())

    tui._age = age


def populate(store) -> None:
    """tests/make_screenshots.py's demo data: three agents, one of them with a two-question group."""
    from herdr_inbox import service

    billing, search, docs = callers()
    service.ask(store, search, "Merge PR #42 with a merge commit or squash?", options=["merge commit", "squash"],
                header="Merge mode", asked_by="reviewer")                                              # Q1
    service.ask_group(store, billing, [
        {"question": "Which database should the invoice store use?", "header": "DB choice", "options": DB},
        {"question": "Which caches should the API warm on deploy?", "header": "Cache", "multi_select": True,
         "options": ["redis", "in-process", "CDN"]}], context="Invoice rows ~1M/day", asked_by="planner",
        title="Invoice storage and caching")                                                            # Q2, Q3
    service.ask(store, docs, "Publish the v2 docs now or after the release?", options=["now", "after"],
                header="Rollout")                                                                       # Q4


async def popup_screens(shots: Shots, work: Path) -> None:
    from herdr_inbox import alerts, service
    from herdr_inbox.tui import InboxApp

    billing, search, docs = callers()
    size = (110, 30)
    src = "InboxApp (the popup) over tests/make_screenshots.py's demo data, focus pane w1:p1"

    store = fresh_store(work, "empty")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await shots.add("popup", src, size, "empty", app, pilot)
    store.close()

    store = fresh_store(work, "demo")
    populate(store)
    # an earlier answer that could not be delivered: its own ⚠ entry in the list
    late = service.ask(store, docs, "Bump the docs build timeout to 90 s?", options=["yes", "no"],
                       header="Timeout")["id"]                                                         # Q5
    store.set_answer(late, "yes", {"choices": ["yes"]})
    store.record_delivery(late, "no live agent in the pane")
    service.save_draft(store, 2, {"choices": ["SQLite"]})
    hold(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await shots.add("popup", src, size, "list", app, pilot)
        await pilot.press("down")
        await shots.add("popup", src, size, "list, cursor on the second row", app, pilot)
        await pilot.press("up", "enter", "left")  # opens on the first blank (Cache); back to DB choice
        await shots.add("popup", src, size, "group: DB choice tab (drafted)", app, pilot)
        await pilot.press("right")
        await shots.add("popup", src, size, "group: Cache tab (multi-select)", app, pilot)
        await pilot.press("space", "down", "space")
        await shots.add("popup", src, size, "group: Cache, two picked", app, pilot)
        await pilot.press("tab")
        await shots.add("popup", src, size, "group: Submit tab", app, pilot)
        await pilot.press("shift+tab", "shift+tab")
        await pilot.press("n", *"no server to run")
        await shots.add("popup", src, size, "group: note box open", app, pilot)
        await pilot.press("escape")
        await pilot.press("d")
        await shots.add("popup", src, size, "dismiss confirmation", app, pilot)
        await pilot.press("escape", "escape")
        await shots.add("popup", src, size, "back on the list", app, pilot)
        await pilot.press("down", "down", "down")
        await shots.add("popup", src, size, "list, cursor on the ⚠ entry", app, pilot)
        await pilot.press("enter")
        await shots.add("popup", src, size, "⚠ entry opened", app, pilot)
    store.close()

    src = "InboxApp, one agent's lone question (opens straight into it)"
    store = fresh_store(work, "single")
    service.ask(store, billing, "Which database should the invoice store use?", options=DB, header="DB choice",
                context="Invoice rows ~1M/day", asked_by="planner")
    hold(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await shots.add("popup", src, size, "single", app, pilot)
        await pilot.press("down")
        await shots.add("popup", src, size, "single, cursor on SQLite", app, pilot)
        await pilot.press("4", "enter", *"MariaDB")
        await shots.add("popup", src, size, "single, typing something else", app, pilot)
    store.close()

    src = "InboxApp, a question whose options carry previews"
    store = fresh_store(work, "preview")
    opts = [{"label": "Small", "description": "2 workers", "preview": '[worker]\nthreads = 2\nmemory = "1Gi"\ncpu = "500m"'},
            {"label": "Large", "description": "8 workers", "preview": '[worker]\nthreads = 8\nmemory = "4Gi"\ncpu = "2"'},
            {"label": "Custom"}]
    service.ask(store, billing, "Which worker profile for the invoice exporter?", options=opts, header="Profile")
    hold(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await shots.add("popup", src, size, "preview", app, pilot)
        await pilot.press("down", "down")
        await shots.add("popup", src, size, "preview, no preview for Custom", app, pilot)
    store.close()

    src = "InboxApp, an alert next to a question"
    store = fresh_store(work, "alerts")
    alerts.raise_alert(store, search, "Deploy needs your eyes", "prod rollout paused at 50%", "urgent", "deployer")
    service.ask(store, docs, "Publish the v2 docs now or after the release?", options=["now", "after"],
                header="Rollout")
    service.ask(store, search, "Keep the old query syntax as an alias?", options=["yes", "no"], header="Syntax")
    hold(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await shots.add("popup", src, size, "alert and questions", app, pilot)
        await pilot.press("up", "enter")
        await shots.add("popup", src, size, "alert opened", app, pilot)
    store.close()

    src = "InboxApp, questions arriving while the list is open (one poll)"
    store = fresh_store(work, "arrivals")
    populate(store)
    hold(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await shots.add("popup", src, size, "before arrivals", app, pilot)
        service.ask(store, search, "Keep the old query syntax as an alias?", options=["yes", "no"], header="Syntax")
        service.ask(store, docs, "Which landing page copy do you prefer?", options=["A", "B"], header="Copy")
        hold(store)
        app.poll()
        await shots.add("popup", src, size, "after one poll", app, pilot)
        service.close(store, 1, "withdrawn")
        app.poll()
        await shots.add("popup", src, size, "a row withdrawn by its agent", app, pilot)
    store.close()

    narrow = (40, 20)
    src = "InboxApp at 40x20, a group of four questions (the tab row scrolls)"
    store = fresh_store(work, "narrow")
    service.ask_group(store, billing, [{"question": f"Question number {i}?", "header": f"Head{i}",
                                        "options": ["yes", "no"]} for i in range(1, 5)], title="Four at once")
    hold(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=narrow) as pilot:
        await shots.add("popup-narrow", src, narrow, "first tab", app, pilot)
        await pilot.press("right", "right", "right")
        await shots.add("popup-narrow", src, narrow, "fourth tab", app, pilot)
        await pilot.press("right")
        await shots.add("popup-narrow", src, narrow, "submit tab", app, pilot)
    store.close()


def capture_screens(out: Path, work: Path) -> list[Path]:
    from tests.conftest import FakeHerdr

    fake = FakeHerdr(work)
    os.environ.update(HERDR_BIN_PATH=str(fake.bin), FAKE_HERDR_LOG=str(fake.log))
    from herdr_inbox import tabs, tui

    # as tests/conftest.py: logic tests press keys at once; the popup sends in its own thread
    tui.ARM_SECONDS = 0.0
    tui.ROW_ARM_SECONDS = 0.0
    tui.LIVE_TAB_LABELS = False
    tui.DETACHED_SEND = False
    tui.ENABLE_BACKGROUND = False
    tui._tab_labels.clear()
    tabs._config_dir.cache_clear()
    hold_clock()
    shots = Shots()
    asyncio.run(popup_screens(shots, work))
    written = []
    for name, doc in shots.files.items():
        doc["source"] += "; TZ=UTC; tools/capture_goldens.py"
        path = out / "screens" / f"{name}.json"
        write(path, doc)
        written.append(path)
    return written


# -- item 5: the delivery decision table ---------------------------------------------------

def write_decisions(path: Path, doc: dict) -> None:
    """One row per line: small diffs when behaviour changes."""
    path.parent.mkdir(parents=True, exist_ok=True)
    head = {k: v for k, v in doc.items() if k != "rows"}
    body = ",\n".join("  " + json.dumps(r, ensure_ascii=False, sort_keys=True, separators=(", ", ": "))
                      for r in doc["rows"])
    text = json.dumps(head, ensure_ascii=False)[:-1] + ', "rows": [\n' + body + "\n]}\n"
    json.loads(text)  # well-formed
    path.write_text(text)


def capture_decisions(out: Path, work: Path, env: dict, tests: list[str] | None) -> list[Path]:
    """The test suite (or only the node ids ``tests``) under tests/decision_recorder.py, in its
    own pytest."""
    table = work / "decisions-raw.json"
    paths = tests or ["tests"]
    res = subprocess.run([sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider",
                          "-p", "tests.decision_recorder", "--decision-table", str(table),
                          "--ignore", "tests/test_goldens.py", *paths],
                         env=env, cwd=ROOT, capture_output=True, text=True, timeout=1800)
    if res.returncode != 0 or not table.exists():
        raise SystemExit(f"the recorded test run failed:\n{res.stdout[-4000:]}\n{res.stderr[-2000:]}")
    rows = json.loads(table.read_text())["rows"]
    doc = {"source": "tests/decision_recorder.py over the inbox test suite (tools/capture_goldens.py); "
                     "see tests/fixtures/README.md",
           "files": sorted({t.split("::", 1)[0] for r in rows for t in r["tests"]}),
           "rows": rows}
    path = out / "decisions.json"
    write_decisions(path, doc)
    return [path]


def capture_schema(out: Path) -> list[Path]:
    """``store._expected_schema()`` at ``store.MIGRATION_REVISION``, sorted, and the SQL of every
    object a migration creates, in creation order (the Rust tests build their databases from it)."""
    import sqlite3

    from herdr_inbox import store

    tables, others = store._expected_schema()
    db = sqlite3.connect(":memory:")
    try:
        store._apply_schema(db)
        sql = [s for (s,) in db.execute(
            "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'"
            " ORDER BY rowid")]
    finally:
        db.close()
    doc = {"source": "store._expected_schema() and store._apply_schema (tools/capture_goldens.py --only schema)",
           "revision": store.MIGRATION_REVISION,
           "tables": {name: sorted(cols) for name, cols in sorted(tables.items())},
           "others": sorted(others),
           "sql": sql}
    path = out / "schema.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc, indent=1) + "\n")
    return [path]


def _generated_screens(count: int = 600, seed: int = 42) -> list[str]:
    """Screens built at random from the pieces both parsers react to (escapes, dim codes, the
    prompt, the placeholder, borders with and without tags, blank and footer lines), a fixed
    seed: the #95 review's differential generator, kept so its cases stay replayed."""
    import random

    rnd = random.Random(seed)
    dim, rst, bold, b22 = "\x1b[2m", "\x1b[0m", "\x1b[1m", "\x1b[22m"
    parts = ["", " ", " ", "❯", "❯ ", "❯ ", 'Try "fix the bug"', 'Try ""', 'Try "a', "hello",
             "日本語", "\U0001f642", "x", dim, rst, bold, b22, "\x1b[38;5;246m", "\x1b[2;3m", "\x1b[m",
             "\x1b[7m", "\x1b[?25l", "\x1b[K", "\x1b]0;title\x07", "\x1b[1;2m", "\x1b[02m", "\x1b[22;2m", "　",
             " ", "\x1c", "\t", "─", "│", "╭", "\x1b[", "\x1b]x"]
    rule = "─"

    def border():
        return rnd.choice([rule * rnd.randint(1, 60), rule * 40 + " node " + rule, rule * 10 + " a b " + rule * 5,
                           rule * 5 + "  x " + rule, rule * 5 + " " + rule + " " + rule, rule * 5 + " " + "y" * 41 + " " + rule,
                           rule * 5 + " " + "y" * 40 + " " + rule, "  " + rule * 20 + "  ", dim + rule * 30 + rst, rule * 3,
                           rule * 2, rule * 5 + " t " + rule + "x", rule * 5 + " t " + rule + " ", rule * 5 + " t " + rule,
                           rule * 5 + " \tt " + rule])

    def line():
        return "".join(rnd.choice(parts) for _ in range(rnd.randint(0, 6)))

    def box_line():
        pre = rnd.choice(["", " ", " ", dim, bold])
        body = rnd.choice(["❯ ", "❯", "❯ " + dim + 'Try "x"' + rst, "❯ " + dim + "T" + rst + 'ry "x"',
                           "❯ " + 'Try "x"', "❯ " + "\x1b[7m" + " " + rst + dim + 'ry "x"' + rst,
                           "❯ draft", "> x", "❯ " + dim + 'Try "x"', line()])
        return pre + body + rnd.choice(["", " ", rst, " "])

    screens = []
    for _ in range(count):
        lines = [line() for _ in range(rnd.randint(0, 6))]
        for _ in range(rnd.randint(0, 3)):
            lines.append(border())
            lines += [box_line()] + [rnd.choice(["", " ", " ", line(), "  "]) for _ in range(rnd.randint(0, 2))]
        if rnd.random() < 0.8:
            lines.append(border())
        lines += [rnd.choice(["", "↑/↓ to select · enter", "  ↑/↓ to select", "? for shortcuts", line()])
                  for _ in range(rnd.randint(0, 3))]
        screens.append("\n".join(lines) + rnd.choice(["", "\n"]))
    return screens


def capture_herdr_screens(out: Path) -> Path:
    """herdr.composer_state, herdr.panel_focused and herdr.live_agent over every screen and
    agent record the tests use (the captured Claude Code screens in tests/fixtures/composer,
    the shapes test_composer_draft and test_delivery_worker build, each rule's edges), each
    screen as the CLI's text mode reads it (universal newlines): what the Rust side
    (src/delivery/screen, src/delivery/pane) must return."""
    import io

    from herdr_inbox import herdr
    from herdr_inbox import store as st
    from tests import test_composer_draft as tcd
    from tests import test_delivery_worker as tdw

    def read(raw: str) -> str:  # subprocess text mode: \r\n and a lone \r read as \n
        return io.StringIO(raw, newline=None).read()

    composer = [(f"{name}.ansi", read(tcd.screen(name)))
                for name in sorted(p.stem for p in (ROOT / "tests" / "fixtures" / "composer").glob("*.ansi"))]
    composer += [(f"variant {line!r}", read(tcd.variant(line))) for line in (
        "❯ \x1b[0m\x1b[2m[Pasted text #1 +3 lines]\x1b[0m\r",
        "❯ \x1b[0m\x1b[2m[Image #1]\x1b[0m\r",
        "❯ \x1b[7mT\x1b[0m\x1b[2mry \"refactor <filepath>\"\x1b[0m\r",
        "❯ \x1b[0m\x1b[2mTry \"x\"\x1b[0m and more\r",
        "❯ Try \"x\"\r",
        "❯ \r")]
    lines = tcd.screen("empty").split("\n")  # test_l1: Shift+Enter first, text on the next box line
    i = next(k for k, line in enumerate(lines) if "❯" in line)
    lines[i:i + 1] = ["❯ \r", "  second line of a draft\r"]
    composer.append(("a blank first line, text on the next", read("\n".join(lines))))
    composer += [(f"no box {text!r}", read(text)) for text in (
        "", "plain output with no input box\n", "\x1b[2m────\x1b[0m\nno prompt\n\x1b[2m────\x1b[0m")]

    # each rule's edges, Python deciding: a box is `top`, the ❯ line, `bottom`
    def box(prompt_line, top="─" * 20, bottom="─" * 20):
        return f"output\n{top}\n{prompt_line}\n{bottom}\n  ? for shortcuts"

    dim, off = "\x1b[2m", "\x1b[0m"
    edges = [
        ("csi erase on the prompt line", box("❯ \x1b[K")),
        ("csi private mode on the prompt line", box("❯ \x1b[?25l")),
        ("osc title on the prompt line", box("❯ \x1b]0;title\x07")),
        ("osc on a border", box("❯ ", top="\x1b]8;;\x07" + "─" * 20)),
        ("csi on a border", box("❯ ", top="\x1b[K" + "─" * 20)),
        ("sgr 22 ends dim", box(f'❯ {dim}Try "\x1b[22mab{dim}c"{off}')),
        ("sgr 2; ends dim", box('❯ \x1b[2;mTry "x"' + off)),
        ("bare sgr ends dim", box(f'❯ {dim}Try "\x1b[mab{dim}c"{off}')),
        ("two lit cells on the placeholder", box(f'❯ \x1b[7mT{off}\x1b[7mr{off}{dim}y "x"{off}')),
        ("placeholder without its closing quote", box(f'❯ {dim}Try "x{off}')),
        ("placeholder of five characters", box(f'❯ {dim}Try "{off}')),
        ("placeholder-like, another word", box(f'❯ {dim}Xry "x"{off}')),
        ("three-dash borders", box("❯ hi", top="───", bottom="───")),
        ("two-dash borders", box("❯ hi", top="──", bottom="──")),
        ("borders with trailing spaces", box("❯ ", top="─" * 20 + "   ", bottom="─" * 20 + "  ")),
        ("a session tag", box("❯ ", top="─" * 20 + " node ─")),
        ("a tag starting with a dash", box("❯ ", top="─" * 20 + " ─x ─")),
        ("a tag starting with a space", box("❯ ", top="─" * 20 + "  x ─")),
        ("a tag with a dash inside", box("❯ ", top="─" * 20 + " x─y ─")),
        ("a tag without its last dashes", box("❯ ", top="─" * 20 + " node ")),
        ("a tag of 41 characters", box("❯ ", top="─" * 20 + " " + "x" * 41 + " ─")),
        ("a tag of 42 characters", box("❯ ", top="─" * 20 + " " + "x" * 42 + " ─")),
        ("adjacent borders", "─" * 20 + "\n" + "─" * 20),
        ("a blank line under the box", box("❯ ") + "\n \n"),
        # #95 review M1: more than two ─── lines; a whitespace-only second box line
        ("rules in the output above a draft", "─" * 20 + "\ntable\n" + "─" * 20 + "\nmore\n" + box("❯ draft text")),
        ("rules in the output above an empty box", "─" * 20 + "\n❯ x\n" + "─" * 20 + "\n" + box("❯ ")),
        ("a whitespace-only second box line", "out\n" + "─" * 20 + "\n❯ \n   \n" + "─" * 20 + "\nfooter"),
    ]
    composer += [(name, read(text)) for name, text in edges]
    panel_edges = [("a blank line, no border", "output\n\n↑/↓ to select"),
                   ("the phrase inside the footer", "─" * 20 + "\n❯\n" + "─" * 20 + "\n  x ↑/↓ to select"),
                   ("a border with trailing spaces", "─" * 20 + "\n❯\n" + "─" * 20 + "   \n  ↑/↓ to select · esc"),
                   ("blank lines above the footer", "─" * 20 + "\n❯\n" + "─" * 20 + "\n\n  \n  ↑/↓ to select · esc"),
                   ("a rule in the output, the footer below the box", "─" * 20 + "\nout\n" + "─" * 20 + "\n❯\n"
                    + "─" * 20 + "\n  ↑/↓ to select")]
    panel = [("normal, the phrase in the body", tdw.SCREEN_NORMAL_WITH_PHRASE_IN_BODY),
             ("panel focused", tdw.SCREEN_PANEL_FOCUSED), ("no input box", "↑/↓ to select"), ("empty", ""),
             *panel_edges]
    generated = _generated_screens()
    real = herdr.run_raw
    try:
        rows = []
        for name, text in composer:
            herdr.run_raw = lambda *a, text=text, **k: (text, None)
            rows.append({"name": name, "screen": text, "composer": herdr.composer_state("w1:p1")})
        for name, text in panel:
            herdr.run_raw = lambda *a, text=read(text), **k: (text, None)
            rows.append({"name": name, "screen": read(text), "panel": herdr.panel_focused("w1:p1")})
        for n, text in enumerate(generated):  # both parsers over every generated screen
            herdr.run_raw = lambda *a, text=text, **k: (text, None)
            rows.append({"name": f"generated {n}", "screen": text, "composer": herdr.composer_state("w1:p1"),
                         "panel": herdr.panel_focused("w1:p1")})
    finally:
        herdr.run_raw = real
    S = "aaaaaaaa-0000-4000-8000-00000000000a"
    full = {"pane_id": "w1:p1", "agent": "claude", "agent_status": "idle", "focused": True,
            "agent_session": {"kind": "id", "value": S}}
    replies = [
        ("a claude agent", {"agent": full}),
        ("no session", {"agent": {**full, "agent_session": None}}),
        ("a session without a value", {"agent": {**full, "agent_session": {"kind": "id"}}}),
        ("an empty session id", {"agent": {**full, "agent_session": {"kind": "id", "value": ""}}}),
        ("no status", {"agent": {k: v for k, v in full.items() if k != "agent_status"}}),
        ("a null status", {"agent": {**full, "agent_status": None}}),
        ("status unknown", {"agent": {**full, "agent_status": "unknown"}}),
        ("a status herdr added later", {"agent": {**full, "agent_status": "starting"}}),
        ("a numeric status", {"agent": {**full, "agent_status": 3}}),
        ("not focused", {"agent": {**full, "focused": False}}),
        ("focused as 1", {"agent": {**full, "focused": 1}}),
        ("focused as 0", {"agent": {**full, "focused": 0}}),
        ("focused null", {"agent": {**full, "focused": None}}),
        ("no kind", {"agent": {k: v for k, v in full.items() if k != "agent"}}),
        ("a codex agent", {"agent": {**full, "agent": "codex"}}),
        ("an empty agent", {"agent": {}}),
        ("no agent", {"agent": None}),
        ("an empty result", {}),
        ("the call failed", None),
    ]
    real_call, real_live = herdr.call, st.live_session
    try:
        st.live_session = lambda reported, *a, **k: f"live-of-{reported}"
        for name, result in replies:
            herdr.call = lambda *a, result=result, **k: (result, None if result is not None else "failed")
            rows.append({"name": name, "result": result, "agent": herdr.live_agent("w1:p1")})
    finally:
        herdr.call, st.live_session = real_call, real_live
    doc = {"source": "herdr.composer_state / herdr.panel_focused / herdr.live_agent (live_session stubbed as "
                     "'live-of-<reported>') (tools/capture_goldens.py --only herdr)",
           "rows": rows}
    path = out / "herdr-screens.json"
    path.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n")
    return path


LANDED_QUESTIONS = [  # (id, question, answer, answered_at)
    (7, "Which region?", "eu", "2026-10-17T10:00:00+00:00"),
    (8, "Squash or merge?\nsecond line", "squash", "2026-10-17T10:00:00+00:00"),
    (27, "Same as before?", "same as Q28", "2026-10-17T10:00:00+00:00"),
    (40, "A long one?", "x" * 4100, "2026-10-17T10:00:00+00:00"),
    (41, "Tabs\tand  spaces?", "a\tb  c\u2028d", "2026-10-17T10:00:00+00:00"),
]


def _landed_texts(qs: dict) -> list[str]:
    """Texts a transcript may hold for the questions: what the inbox writes, and near misses."""
    from herdr_inbox import service

    q7, q8, q27, q40, q41 = (qs[i] for i in (7, 8, 27, 40, 41))
    single = service.delivery_text([q7])
    batch = service.delivery_text([q7, q8, q41])
    return [
        single, batch, service.delivery_text([q8]), service.delivery_text([q27]), service.delivery_text([q41]),
        service._reference(q40), service.delivery_text([q40])[:200],
        "\n\n<pasted_content id=\"ec16\">\n" + batch + "</pasted_content>",
        "  \t" + single, "draft text " + single, "draft text " + service.answer_line(q7),
        "[inbox] Q7 answered", "same as Q7: eu", "[inbox] Q70 answered — Which region?: eu",
        "[inbox] answered — Q27 Same as before?: same as Q28 · Q8 x", single.replace("eu", "us"),
        "<task-notification>Stop hook blocking error from command: " + batch + "\n\nmore</task-notification>",
        "prefix [inbox] Q8 answered — Squash or merge?: squash · [inbox] " + service.answer_line(q7),
        service.delivery_text([q8]).replace(" ", "\n"), "Ünïcödé " + single,
    ]


def _landed_records(text: str) -> list[dict]:
    """Each record type that can carry ``text`` (and some that cannot)."""
    return [
        {"type": "user", "message": {"content": text}},
        {"type": "user", "message": {"content": [{"type": "text", "text": text}, {"type": "image"}]}},
        {"type": "user", "message": {"content": [{"type": "tool_result", "content": text}]}},
        {"type": "queue-operation", "operation": "enqueue", "content": text},
        {"type": "queue-operation", "operation": "dequeue", "content": text},
        {"type": "attachment", "attachment": {"type": "queued_command", "prompt": text}},
        {"type": "attachment", "attachment": {"type": "hook_additional_context", "content": [text]}},
        {"type": "attachment", "attachment": {"type": "hook_additional_context", "content": text}},
        {"type": "assistant", "message": {"content": text}},
    ]


def capture_landed(out: Path) -> list[Path]:
    """``transcript.landed_kinds`` over generated records, one case per (record, time)."""
    import tempfile as _tempfile

    from herdr_inbox import service, store, transcript

    qs = {}
    for qid, question, answer, at in LANDED_QUESTIONS:
        qs[qid] = store.Question(id=qid, created_at=at, pane_id=None, tab_id=None, tab_label=None,
                                 session_id=None, cwd=None, asked_by=None, question=question, options=None,
                                 context=None, norm="", status="answered", answer=answer, answered_at=at,
                                 delivered_at=None, delivery_error=None, acknowledged_at=None, repeat_count=0)
    expected = {qid: (service.expected_segments(q), q.answered_at) for qid, q in qs.items()}
    cases = []
    with _tempfile.TemporaryDirectory() as d:
        path = Path(d) / "t.jsonl"
        for text in _landed_texts(qs):
            for record in _landed_records(text):
                for ts in ("2026-10-17T10:00:05.000Z", "2026-10-17T10:00:00.000Z", "2026-10-17T09:59:59.900Z",
                           "2026-10-17T09:00:00.000Z", "1000000000", None):
                    rec = dict(record, **({"timestamp": ts} if ts else {}))
                    line = json.dumps(rec, ensure_ascii=False)
                    path.write_text(line + "\n")
                    found = transcript.landed_kinds(path, expected)
                    cases.append({"line": line, "found": {str(k): sorted(v) for k, v in sorted(found.items())}})
        for line in ("not json [inbox] Q7 answered", '["[inbox] Q7 answered — Which region?: eu"]'):
            path.write_text(line + "\n")
            found = transcript.landed_kinds(path, expected)
            cases.append({"line": line, "found": {str(k): sorted(v) for k, v in sorted(found.items())}})
    doc = {"source": "transcript.landed_kinds (tools/capture_goldens.py --only landed)",
           "expected": {str(k): {"segments": v[0], "since": v[1]} for k, v in expected.items()},
           "cases": cases}
    path = out / "landed.json"
    path.write_text(json.dumps(doc, ensure_ascii=False, indent=0) + "\n")
    return [path]


STORE_SESSION = "aaaaaaaa-0000-4000-8000-00000000000a"
STORE_FIXED = "2001-01-01T00:00:00+00:00"  # a time a step sets: not "now", so never masked
STORE_OPS = [
    ("insert", {"question": "Which region?", "session_id": STORE_SESSION, "pane_id": "w1:p1", "tab_id": "w1:t1",
                "tab_label": "api", "cwd": "/r", "asked_by": "lane", "options": ["eu", "us"], "header": "Region"}),
    ("insert", {"question": "Deploy which parts?\nSecond line", "session_id": STORE_SESSION, "pane_id": "w1:p1",
                "options": [{"label": "web", "description": "the site"},
                            {"label": "api, v2", "description": "", "preview": "x = 1"}, "docs"],
                "multi_select": True, "group_id": 1, "context": "ctx é \u2028"}),
    ("insert", {"question": "Proceed with the next step?!", "pane_id": "w1:p2"}),
    ("set_group_title", {"group_id": 1, "title": "  Deploy\tplan:\x07 " + "long " * 20}),
    ("set_group_title", {"group_id": 3, "title": " \x01 "}),
    ("find_duplicate", {"norm": "which region", "session_id": STORE_SESSION, "pane_id": "w1:p1"}),
    ("find_duplicate", {"norm": "proceed with the next step", "session_id": None, "pane_id": "w1:p2"}),
    ("set_draft", {"qid": 2, "detail": {"choices": ["web", "api, v2"], "notes": {"web": "n ü"}, "text": None}}),
    ("drafted", {}),
    ("set_draft", {"qid": 2, "detail": None}),
    ("set_answer", {"qid": 1, "answer": "eu", "detail": {"choices": ["eu"], "notes": {}, "text": "ok é"}}),
    ("set_answer", {"qid": 1, "answer": "again", "detail": None}),
    ("set_status", {"qid": 3, "status": "dismissed", "note": "not needed"}),
    ("bump_repeat", {"qid": 1}),
    ("mine", {"session_id": STORE_SESSION, "pane_id": "w1:p1", "include_closed": False}),
    ("mine", {"session_id": None, "pane_id": "w1:p2", "include_closed": True}),
    ("list", {"include_closed": False}),
    ("list", {"include_closed": True}),
    ("group", {"group_id": 1, "include_closed": True}),
    ("open_counts", {}),
    ("known_panes", {}),
    ("unacknowledged_answers", {"session_id": STORE_SESSION, "pane_id": "w1:p1"}),
    ("acknowledge", {"qids": [1, 3]}),
    ("redeliver", {"qids": [1, 2, 3]}),
    ("hide_failed", {"qid": 1}),
    ("get", {"qid": 2}),
    ("get", {"qid": 99}),
    ("acknowledge", {"qids": [1]}),  # a pending error on a seen answer goes
    ("set_draft", {"qid": 2, "detail": {"choices": ["web"], "notes": {}, "text": None}}),
    ("set_answer", {"qid": 2, "answer": "web", "detail": None}),  # the answer clears the draft
    ("mine", {"session_id": None, "pane_id": "w1:p1", "include_closed": True}),  # no session: the pane's own only
    ("insert", {"question": "Ship it?", "pane_id": "w1:p3"}),
    ("insert", {"question": "Ship it?", "pane_id": "w1:p3"}),
    ("set_answer", {"qid": 5, "answer": "yes", "detail": None}),
    ("find_duplicate", {"norm": "ship it", "session_id": None, "pane_id": "w1:p3"}),  # the open one first
    ("claim_delivery", {"qid": 5}),
    ("acknowledge", {"qids": [5]}),  # a claim in flight stays
    # every WHERE and ORDER BY clause decides some step (#79 review M1)
    ("insert", {"question": "Pick a color?", "pane_id": "w1:p4"}),  # 6
    ("set_answer", {"qid": 6, "answer": "red", "detail": None}),
    ("find_duplicate", {"norm": "pick a color", "session_id": None, "pane_id": "w1:p4"}),  # answered, in the hour
    ("sql", {"sql": "UPDATE questions SET answered_at = ? WHERE id = 6", "params": [STORE_FIXED]}),
    ("find_duplicate", {"norm": "pick a color", "session_id": None, "pane_id": "w1:p4"}),  # past ANSWER_REUSE
    ("insert", {"question": "Two of them?", "pane_id": "w1:p5"}),  # 7
    ("insert", {"question": "Two of them?", "pane_id": "w1:p5"}),  # 8
    ("find_duplicate", {"norm": "two of them", "session_id": None, "pane_id": "w1:p5"}),  # the newest open
    ("unacknowledged_answers", {"session_id": STORE_SESSION, "pane_id": "w1:p1"}),  # 1 is acknowledged
    ("group", {"group_id": 1, "include_closed": False}),  # nothing open in it
    ("set_draft", {"qid": 3, "detail": {"choices": ["x"], "notes": {}, "text": None}}),  # closed: no
    ("set_status", {"qid": 3, "status": "withdrawn", "note": "again"}),  # closed: no
    ("set_answer", {"qid": 99, "answer": "x", "detail": None}),
    ("redeliver", {"qids": [99]}),
    ("hide_failed", {"qid": 99}),  # Python writes an orphan question_ext row
    ("insert", {"question": "Nowhere?", "session_id": "bbbbbbbb-0000-4000-8000-00000000000b"}),  # 9, no pane
    ("known_panes", {}),
    ("open_counts", {}),
    ("mine", {"session_id": None, "pane_id": None, "include_closed": True}),  # no owner: nothing
    ("find_duplicate", {"norm": "nowhere", "session_id": None, "pane_id": None}),
    ("sql", {"sql": "UPDATE question_ext SET pending_since = ?, busy_attempts = 2, verified_at = ? WHERE id = 2",
             "params": [STORE_FIXED, STORE_FIXED]}),
    ("redeliver", {"qids": [2]}),  # clears all three
    ("sql", {"sql": "UPDATE questions SET acknowledged_at = ? WHERE id = 2", "params": [STORE_FIXED]}),
    ("acknowledge", {"qids": [2]}),  # keeps the first time
    ("bump_repeat", {"qid": 4}),
    ("bump_repeat", {"qid": 4}),
    ("insert", {"question": "Grouped by zero?", "pane_id": "w1:p6", "group_id": 0}),  # 10: 0 is no group
    ("insert", {"question": "Smile 🙂?", "pane_id": "w1:p6",  # 11: outside the BMP, in every JSON column
                "options": [{"label": "🙂 yes", "description": "😀"}, "no 🙃"]}),
    ("set_draft", {"qid": 11, "detail": {"choices": ["🙂 yes"], "notes": {"🙂 yes": "🎉"}, "text": "𝄞"}}),
    ("set_answer", {"qid": 11, "answer": "🙂 yes",
                    "detail": {"choices": ["🙂 yes"], "notes": {"🙂 yes": "🎉"}, "text": None}}),
    ("get", {"qid": 11}),
    # an empty detail is no detail (Python `if detail`; #79 final pass L2)
    ("insert", {"question": "Empty detail?", "pane_id": "w1:p7"}),  # 12
    ("set_draft", {"qid": 12, "detail": {}}),
    ("set_draft", {"qid": 12, "detail": {"choices": ["a"], "notes": {}, "text": None}}),
    ("set_draft", {"qid": 12, "detail": []}),  # clears it, as None does
    ("set_draft", {"qid": 12, "detail": {"choices": ["a"], "notes": {}, "text": None}}),
    ("set_answer", {"qid": 12, "answer": "a", "detail": {}}),  # no answer_json; the draft goes
]
# The delivery state (store.Store's outcome writes and the piles the worker reads), on a fresh
# database. "$claim:N" stands for the claim the last claim_delivery of question N returned; an
# argument named "<x>_s" is passed to Python as ``x=timedelta(seconds=...)``.
S1, S2, S3 = (f"{c * 8}-0000-4000-8000-00000000000{c}" for c in "123")
_NOW_SQL = "strftime('%Y-%m-%dT%H:%M:%S+00:00', 'now')"
STORE_DELIVERY_OPS = [
    ("insert", {"question": "One?", "session_id": S1, "pane_id": "w1:p1"}),  # 1
    ("insert", {"question": "Two?", "session_id": S1, "pane_id": "w1:p1"}),  # 2
    ("insert", {"question": "Three?", "session_id": S2, "pane_id": "w1:p2"}),  # 3
    ("insert", {"question": "Four?", "pane_id": "w1:p2"}),  # 4, stays open
    ("set_answer", {"qid": 1, "answer": "a1", "detail": None}),
    ("set_answer", {"qid": 2, "answer": "a2", "detail": None}),
    ("set_answer", {"qid": 3, "answer": "a3", "detail": None}),
    ("failed_deliveries", {}),  # answered just now: not yet "never attempted"
    ("undelivered", {"pane_id": "w1:p1", "caps": {}}),
    ("sql", {"sql": "UPDATE questions SET answered_at = ? WHERE id IN (1, 2, 3)", "params": [STORE_FIXED]}),
    ("failed_deliveries", {}),
    ("undelivered", {"pane_id": "w1:p1", "caps": {}}),
    ("claim_delivery", {"qid": 1}),
    ("failed_deliveries", {}),  # a claim in flight is no failure
    ("set_pending", {"qid": 1, "claim": "$claim:1", "reason": "busy: working"}),
    ("get_pending_since", {"qid": 1}),
    ("sql", {"sql": "UPDATE question_ext SET pending_since = ? WHERE id = 1", "params": [STORE_FIXED]}),
    ("claim_delivery", {"qid": 1}),
    ("set_pending", {"qid": 1, "claim": "$claim:1", "reason": "busy: again"}),  # keeps the first time
    ("claim_delivery", {"qid": 1}),
    ("record_busy_miss", {"qid": 1, "claim": "$claim:1", "reason": "busy: typed, unseen"}),
    ("claim_delivery", {"qid": 1}),
    ("record_busy_miss", {"qid": 1, "claim": "$claim:1", "reason": "busy: typed, unseen"}),
    ("busy_attempts", {"qid": 1}),
    ("busy_attempts", {"qid": 99}),
    ("claim_delivery", {"qid": 2}),
    ("set_pending", {"qid": 2, "claim": "$claim:2", "reason": "idle: later"}),  # not busy: no pending_since
    ("set_pending", {"qid": 3, "claim": "sending:2001-01-01T00:00:00+00:00:deadbeef", "reason": "busy: x"}),
    ("record_busy_miss", {"qid": 3, "claim": "sending:2001-01-01T00:00:00+00:00:deadbeef", "reason": "busy: x"}),
    ("get_pending_since", {"qid": 2}),
    ("claim_delivery", {"qid": 3}),
    ("record_delivery", {"qid": 3, "error": "herdr: boom", "claim": "$claim:3"}),
    ("undelivered", {"pane_id": "w1:p2", "caps": {"herdr: ": 1}}),
    ("undelivered", {"pane_id": "w1:p2", "caps": {"herdr: ": 2, "nope: ": 1}}),
    ("undelivered_all", {"caps": {"herdr: ": 1}}),
    ("mark_agent_gone", {"qid": 2, "reason": "pane closed"}),
    ("agent_gone", {"qid": 2}),
    ("agent_gone", {"qid": 1}),
    ("undelivered", {"pane_id": "w1:p1", "caps": {}}),
    ("gone_recent", {"within_s": 3600}),
    ("gone_candidates", {"prefixes": ["agent gone: ", "herdr: "], "cap": 1}),
    ("gone_candidates", {"prefixes": ["herdr: "], "cap": 2}),
    ("move_to_pane", {"qid": 4, "pane_id": "w1:p9", "tab_id": None, "reason": "moved"}),  # open: no
    ("move_to_pane", {"qid": 2, "pane_id": "w1:p9", "tab_id": "w1:t9", "reason": "moved", "tab_label": "new"}),
    ("claim_delivery", {"qid": 2}),
    ("move_to_pane", {"qid": 2, "pane_id": "w1:p8", "tab_id": None, "reason": "again", "claim": "$claim:2"}),
    ("move_to_pane", {"qid": 2, "pane_id": "w1:p7", "tab_id": None, "reason": "stale",
                      "claim": "sending:2001-01-01T00:00:00+00:00:deadbeef"}),
    ("mark_agent_gone", {"qid": 3, "reason": "conversation changed"}),
    ("gone_recent", {"within_s": 3600}),
    ("sql", {"sql": "UPDATE question_ext SET agent_gone_at = ? WHERE id = 3", "params": [STORE_FIXED]}),
    ("gone_recent", {"within_s": 3600}),
    ("archive_gone", {"after_s": 3600}),
    ("archive_gone", {"after_s": 3600}),
    ("mark_seen_elsewhere", {"qid": 3}),
    ("claim_delivery", {"qid": 2}),
    ("mark_seen_elsewhere", {"qid": 2}),  # a claim in flight: not without it
    ("mark_seen_elsewhere", {"qid": 2, "claim": "$claim:2"}),
    ("insert", {"question": "Five?", "session_id": S3, "pane_id": "w1:p3"}),  # 5
    ("set_answer", {"qid": 5, "answer": "a5", "detail": None}),
    ("mark_agent_gone", {"qid": 5, "reason": "x"}),
    ("mark_landed", {"qid": 5, "acknowledge": False}),
    ("sql", {"sql": "UPDATE question_ext SET verified_at = ? WHERE id = 5", "params": [STORE_FIXED]}),
    ("sql", {"sql": "UPDATE questions SET delivered_at = ? WHERE id = 5", "params": [STORE_FIXED]}),
    ("mark_landed", {"qid": 5, "acknowledge": True}),  # keeps the first times
    ("insert", {"question": "Six?", "session_id": S3, "pane_id": "w1:p3"}),  # 6
    ("set_answer", {"qid": 6, "answer": "a6", "detail": None}),
    ("claim_delivery", {"qid": 6}),
    ("mark_landed", {"qid": 6, "acknowledge": True}),  # a claim in flight: no
    ("stuck_answers", {}),
    ("failed_pile", {"within_s": 3600}),
    ("sql", {"sql": "UPDATE questions SET delivery_error = 'herdr: old' WHERE id = 5", "params": []}),
    ("acknowledge", {"qids": [6]}),
    ("clear_seen_errors", {}),
    ("insert", {"question": "Seven?", "session_id": S1, "pane_id": "w1:p1"}),  # 7
    ("set_answer", {"qid": 7, "answer": "a7", "detail": None}),
    ("claim_delivery", {"qid": 7}),
    ("record_hook_delivery", {"qid": 7, "claim": "$claim:7"}),
    ("set_server", {"qid": 7, "server": "/sock/a"}),
    ("server_of", {"qid": 7}),
    ("server_of", {"qid": 99}),
    ("insert", {"question": "Eight?", "session_id": S1, "pane_id": "w1:p1"}),  # 8
    ("set_answer", {"qid": 8, "answer": "a8", "detail": None}),
    ("claim_delivery", {"qid": 8}),
    ("record_hook_delivery", {"qid": 8, "claim": "$claim:8"}),
    ("hook_unverified", {"server": "/sock/a"}),
    ("hook_unverified", {"server": "/sock/b"}),  # 8 has no server: every server's
    ("sql", {"sql": "UPDATE question_ext SET hook_sent_at = ? WHERE id IN (7, 8)", "params": [STORE_FIXED]}),
    ("hook_unseen", {"qid": 7, "sent_at": "2000-01-01T00:00:00+00:00", "reason": "not seen"}),  # not that send
    ("hook_unseen", {"qid": 7, "sent_at": STORE_FIXED, "reason": "not seen"}),
    ("hook_misses", {"qid": 7}),
    ("hook_misses", {"qid": 99}),
    ("hook_unseen", {"qid": 8, "sent_at": STORE_FIXED, "reason": None}),  # no transcript: no miss
    ("stuck_answers", {}),  # 8: sent where no transcript was found, most likely landed: not stuck
    ("hook_verified", {"qid": 8, "acknowledge": True}),
    ("claim_delivery", {"qid": 7}),
    ("record_hook_delivery", {"qid": 7, "claim": "$claim:7"}),
    ("hook_verified", {"qid": 7, "acknowledge": False}),
    ("hook_unverified", {"server": "/sock/a"}),  # 7 is verified now
    ("legacy_check_due", {"qid": 1, "every_s": 60}),
    ("legacy_miss", {"qid": 1}),
    ("legacy_check_due", {"qid": 1, "every_s": 60}),
    ("legacy_miss", {"qid": 1}),
    ("legacy_miss", {"qid": 99}),
    ("sql", {"sql": "UPDATE question_ext SET legacy_checked_at = ? WHERE id = 1", "params": [STORE_FIXED]}),
    ("legacy_check_due", {"qid": 1, "every_s": 60}),
    ("notified", {"qid": 1}),
    ("mark_notified", {"qids": [1, 98]}),
    ("notified", {"qid": 1}),
    ("continued_in", {"session_id": S1}),
    ("record_continuation", {"old": S1, "new": S2}),
    ("record_continuation", {"old": S1, "new": S1}),
    ("record_continuation", {"old": "", "new": S2}),
    ("continued_in", {"session_id": S1}),
    ("record_continuation", {"old": S1, "new": S3}),
    ("continued_in", {"session_id": S1}),
    ("continued_in", {"session_id": None}),
    ("sql", {"sql": f"UPDATE sessions SET watch_seen_at = ?, hook_seen_at = {_NOW_SQL} WHERE session_id = ?",
             "params": [STORE_FIXED, S1]}),
    ("watch_live", {"session_id": S1, "within_s": 60}),
    ("watch_live", {"session_id": S1, "within_s": 30 * 365 * 86400}),
    ("hook_live", {"session_id": S1, "within_s": 60}),
    ("hook_live", {"session_id": S2, "within_s": 60}),
    ("hook_live", {"session_id": None, "within_s": 60}),
    ("failed_pile", {"within_s": 3600}),  # 7 waits ("pending: not seen"): not failed
    ("insert", {"question": "Nine?", "session_id": S2, "pane_id": "w1:p2"}),  # 9
    ("set_answer", {"qid": 9, "answer": "a9", "detail": None}),
    ("claim_delivery", {"qid": 9}),
    ("record_delivery", {"qid": 9, "error": "herdr: down", "claim": "$claim:9"}),
    ("failed_pile", {"within_s": 3600}),
    ("failed_deliveries", {}),  # 3 is hidden (archived)
    ("sql", {"sql": "UPDATE questions SET delivery_error = 'sending:2001-01-01T00:00:00+00:00:deadbeef' WHERE id = 9",
             "params": []}),
    ("failed_deliveries", {}),  # a stale claim: its sender died
    ("failed_pile", {"within_s": 3600}),  # a claim, even a stale one, is not on the pile
    ("mark_agent_gone", {"qid": 9, "reason": "y"}),  # its claim stays
    ("move_to_pane", {"qid": 9, "pane_id": "w1:p5", "tab_id": "w1:t1", "reason": "moved"}),
    ("move_to_pane", {"qid": 9, "pane_id": "w1:p6", "tab_id": "w1:t2", "reason": "moved again"}),
    ("mark_agent_gone", {"qid": 9, "reason": "z"}),
    ("gone_recent", {"within_s": 3600}),
    ("failed_deliveries", {}),
    ("hide_failed", {"qid": 9}),
    ("gone_recent", {"within_s": 3600}),  # hidden: dealt with
    ("failed_deliveries", {}),  # hidden
    ("insert", {"question": "Ten?", "session_id": S2, "pane_id": "w1:p2"}),  # 10
    ("set_answer", {"qid": 10, "answer": "a10", "detail": None}),
    ("claim_delivery", {"qid": 10}),
    ("set_pending", {"qid": 10, "claim": "$claim:10", "reason": "idle: later"}),
    ("failed_pile", {"within_s": 3600}),  # pending: waiting, not failed
    # the guards that keep a seen or delivered answer from being sent again (#82 review M1)
    ("mark_landed", {"qid": 5, "acknowledge": False}),  # 5 is seen: it stays seen
    ("sql", {"sql": "UPDATE question_ext SET hook_sent_at = ? WHERE id = 8", "params": [STORE_FIXED]}),
    ("hook_unseen", {"qid": 8, "sent_at": STORE_FIXED, "reason": "x"}),  # 8 is seen: stays delivered
    ("move_to_pane", {"qid": 5, "pane_id": "w1:p9", "tab_id": None, "reason": "moved"}),  # seen: no
    ("mark_agent_gone", {"qid": 7, "reason": "x"}),  # 7 is delivered: no error
    ("mark_agent_gone", {"qid": 10, "reason": "a"}),
    ("sql", {"sql": "UPDATE question_ext SET agent_gone_at = ? WHERE id = 10", "params": [STORE_FIXED]}),
    ("mark_agent_gone", {"qid": 10, "reason": "b"}),  # keeps the first gone time
    ("gone_candidates", {"prefixes": ["agent gone: "], "cap": 0}),  # 10 is gone already
    ("insert", {"question": "No session?", "pane_id": "w1:p4"}),  # 11
    ("set_answer", {"qid": 11, "answer": "a11", "detail": None}),
    ("mark_agent_gone", {"qid": 11, "reason": "x"}),
    ("gone_recent", {"within_s": 3600}),  # no session: never moved
    ("acknowledge", {"qids": [10]}),
    ("archive_gone", {"after_s": 3600}),  # 10 is seen: not archived
    ("sql", {"sql": "UPDATE questions SET delivery_error = 'herdr: x' WHERE id = 7", "params": []}),
    ("insert", {"question": "Late?", "session_id": S2, "pane_id": "w1:p2"}),  # 12
    ("set_answer", {"qid": 12, "answer": "a12", "detail": None}),
    ("sql", {"sql": "UPDATE questions SET delivered_at = ?, delivery_error = 'herdr: late' WHERE id = 12",
             "params": [STORE_FIXED]}),
    ("stuck_answers", {}),  # 7 is verified: not stuck; 12 was delivered with an error: stuck
]
# Alerts (alerts.py): agent alerts and the blocked-prompt scan, on a fresh database. herdr is
# stubbed at its function boundary: ``server`` is herdr.server_id(), ``agents`` what `agent list`
# returns, ``labels`` the tab labels, ``t`` time.monotonic(). What alerts.py would do to herdr
# (notify, update_markers) is recorded as the step's ``effects``.
_C1 = {"session_id": S1, "pane_id": "w1:p1", "tab_id": "w1:t1", "tab_label": "api"}
_C2 = {"session_id": S2, "pane_id": "w1:p2", "tab_id": None, "tab_label": None}
_C3 = {"session_id": None, "pane_id": "w1:p3", "tab_id": None, "tab_label": None}
_NOPANE = {"session_id": S3, "pane_id": None, "tab_id": None, "tab_label": None}


def _agent(pane, status, tab=None, session=None, kind="claude"):
    a = {"pane_id": pane, "agent_status": status, "agent": kind}
    if tab:
        a["tab_id"] = tab
    if session:
        a["agent_session"] = {"value": session}
    return a


STORE_ALERT_OPS = [
    ("raise_alert", {"caller": _C1, "title": "  Look   at\tthe build ", "message": "  red since 10:00  ",
                     "severity": "urgent", "asked_by": "lane", "server": "/srv/a"}),  # 1
    ("raise_alert", {"caller": _C1, "title": "Look at the build", "message": None, "severity": "info",
                     "asked_by": None, "server": "/srv/a"}),  # the same open one: already_open
    ("raise_alert", {"caller": _C2, "title": "Look at the build", "message": "", "severity": "bogus",
                     "asked_by": None, "server": "/srv/a"}),  # 2: another session; severity -> warning
    ("raise_alert", {"caller": _C3, "title": "x" * 130, "message": "m" * 2100, "severity": "info",
                     "asked_by": None, "server": "/srv/b"}),  # 3: no session; both cut
    ("raise_alert", {"caller": _C1, "title": "   ", "message": None, "severity": "info", "asked_by": None,
                     "server": "/srv/a"}),  # no title
    ("raise_alert", {"caller": _NOPANE, "title": "No pane", "message": None, "severity": "info",
                     "asked_by": None, "server": "/srv/a"}),  # unavailable
    ("open_alerts", {}),
    ("mine", {"caller": _C1}),
    ("mine", {"caller": _C3}),
    ("get", {"aid": 2}),
    ("get", {"aid": 99}),
    ("withdraw", {"caller": _C2, "aid": 1}),  # not its alert
    ("withdraw", {"caller": _C2, "aid": 2}),
    ("withdraw", {"caller": _C2, "aid": 2}),  # not open any more
    ("acknowledge", {"aid": 1}),
    ("acknowledge", {"aid": 1}),
    ("acknowledged_unreported", {"caller": _C1}),
    ("acknowledged_unreported", {"caller": _C1}),  # reported once
    ("raise_alert", {"caller": _C1, "title": "Look at the build", "message": None, "severity": "warning",
                     "asked_by": None, "server": "/srv/a"}),  # 4: the acked one no longer blocks
    ("scan", {"server": "/srv/a", "t": 0.0, "labels": {"w1:t5": "❓2 web"},
              "agents": [_agent("w1:p5", "blocked", "w1:t5", S1), _agent("w1:p6", "working"), {"pane_id": None}]}),
    ("sql", {"sql": "UPDATE blocked_panes SET since = ? WHERE pane_id = 'w1:p5'", "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 1.0, "labels": {"w1:t5": "❓2 web"},
              "agents": [_agent("w1:p5", "blocked", "w1:t5", S1), _agent("w1:p7", "blocked", kind="codex")]}),  # 5
    ("scan", {"server": "/srv/a", "t": 2.0, "labels": {},
              "agents": [_agent("w1:p5", "blocked", "w1:t5", S1), _agent("w1:p7", "blocked")]}),  # no second one
    ("withdraw", {"caller": _C1, "aid": 5}),  # open, its session's, but a blocked alert: not the agent's
    ("sql", {"sql": "UPDATE blocked_panes SET since = ? WHERE pane_id = 'w1:p7'", "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 3.0, "labels": {},
              "agents": [_agent("w1:p5", "working"), _agent("w1:p7", "blocked")]}),  # 6 for p7; p5 flickers
    ("scan", {"server": "/srv/a", "t": 5.0, "labels": {},
              "agents": [_agent("w1:p5", "working"), _agent("w1:p7", "blocked")]}),  # two ticks, too soon
    ("scan", {"server": "/srv/a", "t": 14.0, "labels": {},
              "agents": [_agent("w1:p5", "working"), _agent("w1:p7", "blocked")]}),  # p5 clears
    ("acknowledge", {"aid": 6}),  # the operator dismissed p7's
    ("scan", {"server": "/srv/a", "t": 15.0, "labels": {}, "agents": [_agent("w1:p7", "blocked")]}),  # held
    ("scan", {"server": "/srv/b", "t": 16.0, "labels": {}, "agents": [_agent("w1:p7", "blocked")]}),  # other server
    ("scan", {"server": "/srv/a", "t": 17.0, "labels": {}, "agents": []}),  # p7 gone: its episode ends
    ("scan", {"server": "/srv/a", "t": 18.0, "labels": {}, "agents": [_agent("w1:p7", "blocked")]}),
    ("sql", {"sql": "UPDATE blocked_panes SET since = ? WHERE pane_id = 'w1:p7' AND herdr_server = '/srv/a'",
             "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 19.0, "labels": {}, "agents": [_agent("w1:p7", "blocked")]}),  # alerts again
    ("scan_failed", {"server": "/srv/a"}),  # herdr did not answer: nothing changes
    ("scan", {"server": "/srv/b", "t": 20.0, "labels": {}, "agents": []}),  # /srv/a's alerts are not /srv/b's
    ("mine", {"caller": _C1}),  # agent alerts only: not the blocked one of its session
    ("raise_alert", {"caller": {"session_id": S3, "pane_id": "w1:p3", "tab_id": None, "tab_label": None},
                     "title": "Same pane, a session", "message": None, "severity": "info", "asked_by": None,
                     "server": "/srv/a"}),  # 8
    ("withdraw", {"caller": _C3, "aid": 8}),  # the pane's session-less asker does not own it
    # p8 flickers: one sighting not blocked, blocked again (the memory resets), then not blocked
    ("scan", {"server": "/srv/a", "t": 30.0, "labels": {}, "agents": [_agent("w1:p8", "blocked")]}),
    ("scan", {"server": "/srv/a", "t": 31.0, "labels": {}, "agents": [_agent("w1:p8", "working")]}),
    ("scan", {"server": "/srv/a", "t": 32.0, "labels": {}, "agents": [_agent("w1:p8", "blocked")]}),
    ("scan", {"server": "/srv/a", "t": 50.0, "labels": {}, "agents": [_agent("w1:p8", "working")]}),  # stays
    ("open_alerts", {}),
    ("insert", {"question": "In play?", "session_id": S1, "pane_id": "w1:p1"}),
    ("insert", {"question": "Answered, unseen", "session_id": S2, "pane_id": "w1:p2"}),
    ("set_answer", {"qid": 2, "answer": "x", "detail": None}),
    ("insert", {"question": "Done", "session_id": S3, "pane_id": "w1:p3"}),
    ("set_status", {"qid": 3, "status": "dismissed", "note": None}),
    ("raise_alert", {"caller": {"session_id": "44444444-0000-4000-8000-000000000004", "pane_id": "w1:p4",
                                "tab_id": None, "tab_label": None}, "title": "Closed later", "message": None,
                     "severity": "info", "asked_by": None, "server": "/srv/a"}),  # 9
    ("acknowledge", {"aid": 9}),
    ("insert", {"question": "Seen", "session_id": "55555555-0000-4000-8000-000000000005", "pane_id": "w1:p5"}),
    ("set_answer", {"qid": 4, "answer": "x", "detail": None}),
    ("acknowledge_answers", {"qids": [4]}),
    ("sessions_in_play", {}),  # not a closed alert's session, not a seen answer's
    # #83 review M1
    ("scan", {"server": "/srv/a", "t": 100.0, "labels": {}, "agents": [_agent("w1:p9", "blocked", "w1:t9")]}),
    ("sql", {"sql": "UPDATE blocked_panes SET since = ? WHERE pane_id = 'w1:p9'", "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 101.0, "labels": {"w1:t9": "❓2 "},  # strips to "": the pane names it
              "agents": [_agent("w1:p9", "blocked", "w1:t9")]}),
    ("sql", {"sql": "INSERT INTO alerts (created_at, kind, severity, title, pane_id, status) VALUES "
                    "(?, 'blocked', 'warning', 'legacy', 'w1:p10', 'open')", "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 102.0, "labels": {}, "agents": [_agent("w1:p9", "blocked", "w1:t9")]}),
    ("scan", {"server": "/srv/a", "t": 110.0, "labels": {}, "agents": [_agent("w1:p11", "blocked")]}),
    ("scan", {"server": "/srv/a", "t": 111.0, "labels": {}, "agents": [_agent("w1:p11", "working")]}),
    ("scan", {"server": "/srv/a", "t": 121.0, "labels": {}, "agents": [_agent("w1:p11", "working")]}),  # 10 s, 2 scans
    ("scan", {"server": "/srv/a", "t": 130.0, "labels": {}, "agents": [_agent("w1:p12", "blocked", None, S1)]}),
    ("sql", {"sql": "UPDATE blocked_panes SET since = ? WHERE pane_id = 'w1:p12'", "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 131.0, "labels": {}, "agents": [_agent("w1:p12", "blocked", None, S1)]}),
    ("raise_alert", {"caller": _C1, "title": "w1:p12 is waiting for you", "message": None, "severity": "info",
                     "asked_by": None, "server": "/srv/a"}),  # only agent alerts are duplicates
    # every remaining condition (#83 review: one step per predicate)
    ("raise_alert", {"caller": {**_C1, "pane_id": ""}, "title": "Empty pane", "message": None, "severity": "info",
                     "asked_by": None, "server": "/srv/a"}),  # "" is no pane: unavailable
    ("raise_alert", {"caller": {**_C1, "tab_label": ""}, "title": "Empty label", "message": None, "severity": "info",
                     "asked_by": None, "server": "/srv/a"}),  # 14: an empty label names the pane
    ("withdraw", {"caller": {"session_id": None, "pane_id": "w1:p4", "tab_id": None, "tab_label": None},
                  "aid": 3}),  # session-less on both sides, another pane: not its
    ("acknowledge", {"aid": 12}),
    ("acknowledged_unreported", {"caller": _C1}),  # a blocked alert of its session is not reported
    ("scan", {"server": "/srv/a", "t": 200.0, "labels": {}, "agents": [_agent("w1:p1", "blocked")]}),
    ("sql", {"sql": "UPDATE blocked_panes SET since = ? WHERE pane_id = 'w1:p1'", "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 201.0, "labels": {},  # A1 (an acked agent alert) dismisses nothing
              "agents": [_agent("w1:p1", "blocked")]}),
    ("sql", {"sql": "UPDATE alerts SET closed_at = ? WHERE id = 12", "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 202.0, "labels": {}, "agents": [_agent("w1:p1", "blocked")]}),  # keeps it
    ("scan", {"server": "/srv/a", "t": 210.0, "labels": {"w1:t16": "", "w1:t19b": "web19"}, "agents": [
        _agent("w1:p16", "blocked", "w1:t16"), {"pane_id": "w1:p17", "agent_status": "blocked", "tab_id": ""},
        _agent("w1:p18", "idle"), _agent("w1:p19", "blocked", "w1:t19a"), _agent("w1:p19", "blocked", "w1:t19b"),
        {"pane_id": "", "agent_status": "blocked"}]}),
    ("sql", {"sql": "UPDATE blocked_panes SET since = ? WHERE pane_id IN ('w1:p16', 'w1:p17', 'w1:p18', 'w1:p19')",
             "params": [STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 211.0, "labels": {"w1:t16": "", "w1:t19b": "web19"}, "agents": [
        _agent("w1:p16", "blocked", "w1:t16"), {"pane_id": "w1:p17", "agent_status": "blocked", "tab_id": ""},
        _agent("w1:p18", "idle"), _agent("w1:p19", "blocked", "w1:t19a"), _agent("w1:p19", "blocked", "w1:t19b"),
        {"pane_id": "", "agent_status": "blocked"}]}),  # 16: no label; 17: tab ""; 18 idle; 19: the last entry
    ("sql", {"sql": "INSERT INTO alerts (created_at, kind, severity, title, pane_id, herdr_server, status) VALUES "
                    "(?, 'blocked', 'warning', 'twice a', 'w1:p20', '/srv/a', 'open'), "
                    "(?, 'blocked', 'warning', 'twice b', 'w1:p20', '/srv/a', 'open')",
             "params": [STORE_FIXED, STORE_FIXED]}),
    ("scan", {"server": "/srv/a", "t": 212.0, "labels": {}, "agents": []}),  # only the later one of p20 clears
    ("insert", {"question": "Open only", "session_id": "66666666-0000-4000-8000-000000000006", "pane_id": "w1:p6"}),
    ("sessions_in_play", {}),  # an open question alone puts its session in play
]
# The remote mirror (remote.py, the puller's side). ``pull`` runs remote.pull with its ssh stubbed:
# ``paths_error`` makes the paths probe fail, ``code``/``stdout``/``stderr`` are what the export call
# returns (``export`` is turned into its JSON), ``now`` is remote._now() in unix seconds. The
# call's cursor and full flag and every notification are recorded as the step's ``effects``.
_T0 = 978307200  # 2001-01-01T00:00:00+00:00
_DBA, _DBB = "a" * 32, "b" * 32
_M1 = {"id": "m1", "label": "box", "target": "box.lan"}
_M2 = {"id": "m2", "label": None, "target": "other.lan"}
_NV = "not verified: herdr accepted the text but it never reached the conversation"
# What the remote's paths probe and `herdr plugin list --json` print, unless a step says otherwise.
_PROBE_OUT = "/home/u/.local/bin/uv\n/usr/local/bin/herdr\nmissing:claude\n"
_PLUGINS_OUT = json.dumps({"result": {"plugins": [
    {"plugin_id": "other.plugin", "plugin_root": "/x"},
    {"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": "/home/u/.config/herdr/plugins/inbox"}]}})


def _rq(i, status="open", group=None, **kw):
    row = {"id": i, "qkey": f"{i:08x}", "group": i if group is None else group, "status": status,
           "question": f"Question {i}?\nsecond line", "options": ["yes", "no"]}
    row.update(kw)
    return row


def _ra(i, status="open", **kw):
    a = {"id": i, "akey": f"{i:08x}aa", "status": status, "kind": "agent", "severity": "warning",
         "title": f"Alert {i}", "tab_label": "web", "created_at": "2001-01-01T00:00:00+00:00"}
    a.update(kw)
    return a


def _ex(db, seq, questions, full=False, ids=None, gone=None, alerts=None, **kw):
    out = {"version": "0.10.6", "format": 1, "db_id": db, "seq": seq, "full": full, "questions": questions}
    if ids is not None:
        out["ids"] = ids
    if gone is not None:
        out["gone"] = gone
    if alerts is not None:
        out["alerts"] = alerts
    out.update(kw)
    return out


STORE_MIRROR_OPS = [
    ("pull", {"machine": _M1, "now": _T0, "probe": "/home/u/.local/bin/uv\nmissing:herdr\n"}),  # no herdr there
    ("pull", {"machine": _M1, "now": _T0 + 1}),  # backing off: no call
    ("pull", {"machine": _M1, "now": _T0 + 6, "code": 2}),  # no `export` there
    ("pull", {"machine": _M1, "now": _T0 + 20, "code": 255, "stderr": "  ssh: connect refused\n"}),
    ("pull", {"machine": _M1, "now": _T0 + 60, "code": 1, "stderr": "  "}),
    ("pull", {"machine": _M1, "now": _T0 + 200, "export": {**_ex(_DBA, 1, []), "format": 2, "version": "0.11.0"}}),
    ("pull", {"machine": _M1, "now": _T0 + 400, "export": _ex("XYZ", 1, [])}),  # bad header
    ("pull", {"machine": _M1, "now": _T0 + 800, "export": _ex(_DBA, 5, [
        _rq(1, group=1, tab_label="api"),
        _rq(2, "answered", delivery_error=_NV + " (2 tries)", delivery_attempts=2, answer="eu"),
        _rq(3, group=1, pane_id="w1:p3"),
        _rq(4, pane_id="w1:p4", tab_label=""),
        _rq(6, "answered", agent_gone=True),
        _rq(7, "answered", delivery_error=_NV, delivery_attempts=1),
        {**_rq(8), "qkey": "NOT-HEX"},
    ], full=True, ids=[1, 2, 3, 4, 6, 7], alerts=[_ra(1), _ra(2, "acked")])}),
    ("open_groups", {}),
    ("open_alerts", {}),
    ("alert_by_key", {"key": "m1|1"}),
    ("alert_by_key", {"key": "m1|2"}),  # acked: not open
    ("group_by_key", {"key": f"m1|{_DBA}|1"}),
    ("group_by_key", {"key": f"m1|{_DBA}|99"}),
    ("problem_rows", {}),
    ("sql", {"sql": "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, created_at,"
                    " expires_at, state) VALUES ('r1', 'm1', ?, 3, '00000003', 'answer', ?, ?, 'queued'),"
                    " ('r2', 'm1', ?, 1, '00000001aa', 'ack_alert', ?, ?, 'queued'),"
                    " ('r3', 'm1', ?, 4, '00000004', 'hide', ?, ?, 'queued')",
             "params": [_DBA, STORE_FIXED, STORE_FIXED] * 3}),
    ("pull", {"machine": _M1, "now": _T0 + 801, "export": _ex(_DBA, 9, [
        _rq(1, "answered", group=1, tab_label="api", answer="yes"),
        _rq(5, group=5, tab_label="db"),
        _rq(7, "answered", delivery_error=_NV, delivery_attempts=2),  # now a problem
    ], gone=[4], alerts=[_ra(1), _ra(3, kind="blocked", severity="urgent", tab_label=None)])}),
    ("open_groups", {}),  # 3 has an answer queued here
    ("open_alerts", {}),  # 1 is being acknowledged: shown acked
    ("problem_rows", {}),
    ("clear_problem", {"machine_id": "m1", "remote_id": 2, "now": _T0 + 802}),
    ("problem_rows", {}),
    ("notify_problems", {"machine_id": "m1", "label": "box", "now": _T0 + 802}),  # each once
    ("sql", {"sql": "UPDATE remote_machines SET pulls = 60 WHERE machine_id = 'm1'", "params": []}),
    ("pull", {"machine": _M1, "now": _T0 + 803, "export": _ex(_DBA, 10, [], full=True, ids=[1, 2, 3])}),
    ("pull", {"machine": _M1, "now": _T0 + 804, "export": _ex(_DBB, 1, [_rq(1, tab_label="new")])}),  # reset
    ("pull", {"machine": {**_M1, "label": ""}, "now": _T0 + 805, "export": _ex(_DBB, 2, [_rq(2)])}),  # no label
    ("pull", {"machine": _M2, "now": _T0 + 806, "export": _ex("c" * 32, 3, [_rq(1, tab_label="x")], full=True,
                                                                ids=[1], alerts=[_ra(9, tab_label=None)])}),
    ("open_alerts", {}),  # m2 (no label) sorts first
    ("open_groups", {}),
    ("alert_by_key", {"key": "m2|9"}),
    # one step per remaining predicate (#83's rule, applied before review)
    ("alert_by_key", {"key": "m1|9"}),  # m2's alert 9, not m1's
    ("pull", {"machine": {"id": "m4", "label": "four", "target": "t4"}, "now": _T0 + 900,
              "plugins": "[]"}),  # json.loads -> list: list.get is an AttributeError
    ("pull", {"machine": {"id": "m4", "label": "four", "target": "t4"}, "now": _T0 + 905,  # exactly backoff_until
              "export": _ex("d" * 32, 1, [{**_rq(1), "id": True}], full=True, ids=[1])}),  # a bool id: QTrue
    ("sql", {"sql": "UPDATE remote_machines SET db_id = '', pulls = 5 WHERE machine_id = 'm2'", "params": []}),
    ("pull", {"machine": _M2, "now": _T0 + 906, "export": _ex("c" * 32, 4, [  # '' is no cursor: a full pull
        _rq(1, tab_label="x"), _rq(2, "answered", delivery_error="herdr: down", delivery_attempts=5),
        _rq(3, "answered", delivery_error=_NV, delivery_attempts=3)], full=True, ids=[1, 2, 3],
        alerts=[_ra(9, tab_label=None)])}),
    ("sql", {"sql": "UPDATE remote_questions SET issue_notified_at = NULL WHERE machine_id = 'm2'", "params": []}),
    ("notify_problems", {"machine_id": "m1", "label": "box", "now": _T0 + 907}),  # m2's are not m1's
    ("sql", {"sql": "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, created_at,"
                    " expires_at, state) VALUES ('r4', 'm1', ?, 2, '00000002', 'dismiss', ?, ?, 'queued'),"
                    " ('r5', 'm1', ?, 7, '00000007', 'answer', ?, ?, 'done'),"
                    " ('r6', 'm1', ?, 3, '00000003aa', 'ack_alert', ?, ?, 'done'),"
                    " ('r7', 'm1', ?, 4, '00000004aa', 'ack_alert', ?, ?, 'queued')",
             "params": [_DBB, STORE_FIXED, STORE_FIXED] * 4}),
    ("pull", {"machine": {**_M1, "label": ""}, "now": _T0 + 908, "export": _ex(_DBB, 3, [
        _rq(1, tab_label="new"), _rq(2), _rq(7), _rq(8, "answered", delivery_error=_NV, delivery_attempts=2)],
        alerts=[_ra(3), _ra(4, "withdrawn")])}),  # 1 seen again: no notice; 2 and 7 queued/done here
    ("open_groups", {}),
    ("open_alerts", {}),
    ("problem_rows", {}),  # m1 has no label now: its id names it
    ("sql", {"sql": "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at)"
                    " VALUES ('m1', ?, 50, '00000032', 'open', ?, ?)",
             "params": ["e" * 32, '{"id": 50, "group": 1, "status": "open", "question": "Other db?"}', STORE_FIXED]}),
    ("open_groups", {}),  # group 1 of another db_id is another group
    ("pull", {"machine": {**_M1, "label": ""}, "now": _T0 + 909, "export": _ex("f" * 32, 1, [])}),  # reset: r4, r7 fail
    ("pull", {"machine": {**_M1, "label": ""}, "now": _T0 + 910, "export": _ex("0" * 32, 1, [])}),  # reset, nothing queued
    # the paths probe and its cache (#86 review M2): each failure, then the paths cached for good
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1000,
              "probe": "  /opt/uv  \r\nnot/absolute/herdr\n/bad path/herdr\n"}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1100, "plugins": "not json"}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1200,
              "plugins": json.dumps({"result": {"plugins": [{"plugin_id": "leavemyyard.herdr-inbox",
                                                             "plugin_root": "relative/root"}]}})}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1300,
              "plugins": json.dumps({"result": {"plugins": [5, {"plugin_id": "leavemyyard.herdr-inbox"}]}})}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1400,
              "plugins": json.dumps({"result": [1]})}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1500,
              "plugins": json.dumps({"result": {"plugins": 7}})}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1600,
              "plugins": json.dumps({"result": {"plugins": [{"plugin_id": "leavemyyard.herdr-inbox"}]}})}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1700,
              "probe": "  /a/uv\n /b/uv \n\t/c/herdr\n", "export": _ex("5" * 32, 1, [
                  _rq(1, "answered", delivery_error=_NV, delivery_attempts=2, verified_at=STORE_FIXED),
                  _rq(2, "answered", delivery_error=_NV, delivery_attempts=2, acknowledged_at=STORE_FIXED)])}),  # cached now
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1701,
              "probe": "", "export": _ex("5" * 32, 2, [])}),  # cached: no probe at all
    ("problem_rows", {}),  # m5's verified answer is no problem
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1702, "code": 1,
              "stderr": "e" * 100 + "\n" + "x" * 350 + "\n"}),  # only the last 300 characters
    ("sql", {"sql": "UPDATE remote_machines SET uv = NULL, backoff_until = NULL WHERE machine_id = 'm5'", "params": []}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1703, "plugins": ""}),  # re-probed; '' -> {}
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1800,
              "plugins": json.dumps({"result": {"plugins": "ab"}})}),  # a string: its characters
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 1900,
              "plugins": json.dumps({"result": {"plugins": [{"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": "/"}]}})}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 2000,
              "plugins": json.dumps({"result": {"plugins": [{"plugin_id": "leavemyyard.herdr-inbox",
                                                             "plugin_root": "/" + "a" * 501}]}})}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 2100, "export": _ex("5" * 32, 3, [])}),
    ("sql", {"sql": "UPDATE remote_machines SET plugin_root = '' WHERE machine_id = 'm5'", "params": []}),
    ("pull", {"machine": {"id": "m5", "label": "five", "target": "t5"}, "now": _T0 + 2101,
              "plugins": json.dumps({"result": {"plugins": [{"plugin_id": "leavemyyard.herdr-inbox",
                                                             "plugin_root": "/" + "b" * 500}]}}),
              "export": _ex("5" * 32, 4, [])}),  # '' is not cached: probed again; 500 characters is fine
    # the ssh target and the fixed command lines are checked before any ssh runs (remote.ssh, call,
    # remote_command): an unsafe target would be read as an ssh option on THIS machine
    *[("pull", {"machine": {"id": f"t{i}", "label": None, "target": target}, "now": _T0 + 3000})
      for i, target in enumerate(["-oProxyCommand=touch /x", "a b", "a\tb", "a\x00b", "h\u0085", "host;rm",
                                  "user@host=1", "x" * 254, "", "é.lan", None])],
    *[("pull", {"machine": {"id": f"ok{i}", "label": None, "target": target}, "now": _T0 + 3000,
                "export": _ex("6" * 32, 1, [])})
      for i, target in enumerate(["x" * 253, "user@host:22", "[::1]", "a-b_c.d@e"])],
    ("sql", {"sql": "UPDATE remote_machines SET target = '-p22' WHERE machine_id = 'ok1'", "params": []}),
    ("pull", {"machine": {"id": "ok1", "label": None, "target": "-p22"}, "now": _T0 + 3100}),  # cached paths: call() refuses
    ("sql", {"sql": "UPDATE remote_machines SET uv = 'uv' WHERE machine_id = 'ok2'", "params": []}),
    ("pull", {"machine": {"id": "ok2", "label": None, "target": "[::1]"}, "now": _T0 + 3100}),  # bad uv
    ("sql", {"sql": "UPDATE remote_machines SET herdr = '/h;x' WHERE machine_id = 'ok3'", "params": []}),
    ("pull", {"machine": {"id": "ok3", "label": None, "target": "a-b_c.d@e"}, "now": _T0 + 3100}),  # bad herdr
    ("sql", {"sql": "UPDATE remote_machines SET plugin_root = '/r oot' WHERE machine_id = 'ok0'", "params": []}),
    ("pull", {"machine": {"id": "ok0", "label": None, "target": "x" * 253}, "now": _T0 + 3100}),  # bad plugin_root
]
# The outbox (remote.py: enqueue, flush, the sweep, the popup's calls), on a fresh database whose
# mirror comes from one pull. remote._now is ``now`` (unix seconds, kept until a step sets it),
# each `inbox apply` answers with the next of the step's ``replies``, uuid4 counts up (request ids
# 000…001, 000…002, …). Each apply call also records the row's sending_at while it runs.
_T1 = _T0 + 10_000
_DBX = "a" * 32


def _q(i, status="open"):
    return {"id": i, "qkey": f"{i:08x}", "_db_id": _DBX, "status": status}


def _ob(rid, mid, remote_id, action, created, expires, attempts=0, db=_DBX, qkey=None, state="queued",
        sending=None, next_try=None):
    return {"sql": "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, created_at,"
                   " expires_at, attempts, state, sending_at, next_try_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            "params": [rid, mid, db, remote_id, qkey or f"{remote_id:08x}", action, created, expires, attempts, state,
                       sending, next_try]}


def _iso_at(t):
    from datetime import datetime, timezone

    return datetime.fromtimestamp(t, tz=timezone.utc).isoformat(timespec="seconds")


STORE_OUTBOX_OPS = [
    ("pull", {"machine": _M1, "now": _T1, "export": _ex(_DBX, 1, [
        _rq(1), _rq(2), _rq(3, "answered"), _rq(4)], full=True, ids=[1, 2, 3, 4],
        alerts=[_ra(1), _ra(2)])}),
    ("enqueue", {"machine_id": "m1", "data": _q(1), "action": "answer",
                 "detail": {"choices": ["yes"], "notes": {}, "text": None}}),
    ("enqueue", {"machine_id": "m1", "data": _q(1), "action": "dismiss", "detail": None}),  # same question: waits
    ("enqueue", {"machine_id": "m1", "data": _q(99), "action": "answer", "detail": None}),  # gone
    ("enqueue", {"machine_id": "m1", "data": {**_q(2), "qkey": "ffffffff"}, "action": "answer", "detail": None}),
    ("enqueue", {"machine_id": "m1", "data": {**_q(2), "_db_id": "b" * 32}, "action": "answer", "detail": None}),
    ("enqueue", {"machine_id": "m1", "data": _q(3, "answered"), "action": "deliver", "detail": {}}),  # {}: no detail
    ("enqueue", {"machine_id": "m1", "data": _q(4), "action": "hide", "detail": None}),
    ("enqueue_alert_ack", {"machine_id": "m1", "data": {"id": 1, "akey": "00000001aa", "_db_id": _DBX}}),
    ("enqueue_alert_ack", {"machine_id": "m1", "data": {"id": 2, "akey": "ffffffffaa", "_db_id": _DBX}}),
    ("enqueue_alert_ack", {"machine_id": "m1", "data": {"id": 2, "akey": "00000002aa", "_db_id": "b" * 32}}),
    ("outbox_rows", {}),
    ("flush", {"machine_id": "m1", "now": _T1 + 1, "replies": [
        {"reply": {"outcome": "applied"}},
        {"reply": {"outcome": "rejected", "reason": "already delivered\nby hand"}},
        {"stdout": "oops", "stderr": "e" * 250, "code": 1},
        {"reply": {"outcome": "already_applied"}}]}),
    ("flush", {"machine_id": "m1", "now": _T1 + 2, "replies": [{"code": 2}]}),  # the dismiss: no `apply` there
    ("flush", {"machine_id": "m1", "now": _T1 + 5, "replies": []}),  # the retry's next_try_at is T+6
    ("flush", {"machine_id": "m1", "now": _T1 + 6, "replies": [{"reply": {"outcome": "rejected", "reason": 5}}]}),
    ("sql", _ob("r06", "m1", 3, "dismiss", _iso_at(_T1 + 7), _iso_at(_T1 + 9999))),
    ("sql", _ob("r07", "m1", 99, "answer", _iso_at(_T1 + 7), _iso_at(_T1 + 9999))),
    ("sql", _ob("r08", "m1", 4, "deliver", _iso_at(_T1 + 7), _iso_at(_T1 + 9999), db="c" * 32)),
    ("sql", _ob("r09", "m1", 2, "hide", _iso_at(_T1 + 7), _iso_at(_T1 + 9999))),
    ("sql", _ob("r10", "m1", 2, "answer", _iso_at(_T1 + 7), _iso_at(_T1 + 9999), qkey="eeeeeeee")),  # behind r09
    ("sql", _ob("r11", "m1", 1, "ack_alert", _iso_at(_T1 + 7), _iso_at(_T1 + 9999), qkey="00000001aa")),  # alert 1, not Q1
    ("flush", {"machine_id": "m1", "now": _T1 + 8, "replies": [{"reply": {"outcome": "applied"}},
                                                                {"reply": ["not", "an", "object"]}]}),
    ("flush", {"machine_id": "m1", "now": _T1 + 20, "replies": [{"reply": {"outcome": "applied"}}]}),  # r10 now
    ("sql", _ob("r21", "m1", 21, "answer", _iso_at(_T1 + 21), _iso_at(_T1 + 22))),
    ("sql", _ob("r22", "m1", 22, "answer", _iso_at(_T1 + 21), _iso_at(_T1 + 22), attempts=2)),
    ("sql", _ob("r23", "m1", 23, "dismiss", _iso_at(_T1 + 21), _iso_at(_T1 + 22), attempts=1)),
    ("sql", _ob("r24", "m1", 24, "answer", _iso_at(_T1 + 21), _iso_at(_T1 + 22), attempts=1)),
    ("sql", _ob("r25", "m1", 25, "ack_alert", _iso_at(_T1 + 21), _iso_at(_T1 + 22), attempts=1)),
    ("sql", _ob("r26", "m1", 26, "answer", _iso_at(_T1 + 21), _iso_at(_T1 + 22), attempts=1)),
    ("flush", {"machine_id": "m1", "now": _T1 + 30, "replies": [
        {"reply": {"outcome": "recorded", "state": "applied"}},
        {"reply": {"outcome": "recorded", "state": "rejected", "reason": "closed there"}},
        {"reply": {"outcome": "expired"}},
        {"reply": {"outcome": "rejected"}},
        {"code": 255, "stderr": "unreachable"}]}),  # r26: no reply, left as it is
    ("sql", _ob("r31", "m1", 31, "answer", _iso_at(_T1 + 31), _iso_at(_T1 + 32), attempts=1)),
    ("sql", _ob("r32", "m1", 2, "answer", _iso_at(_T1 + 33), _iso_at(_T1 + 9999))),
    ("flush", {"machine_id": "m1", "now": _T1 + 40, "replies": [{"code": 255}, {"reply": [1]}]}),  # r31's status: a list
    ("sql", {"sql": "UPDATE remote_machines SET target = 'x;y' WHERE machine_id = 'm1'", "params": []}),
    ("flush", {"machine_id": "m1", "now": _T1 + 41, "replies": []}),  # call() refuses the target
    ("sql", {"sql": "INSERT INTO remote_machines (machine_id, label, target, last_error, last_ok_at)"
                    " VALUES ('m8', 'eight', 't8', 'down', ?), ('m9', NULL, 't9', NULL, ?), ('m10', 'ten', 't10', NULL, NULL)",
             "params": [_iso_at(_T1), _iso_at(_T1)]}),
    ("sql", _ob("r41", "m10", 1, "answer", _iso_at(_T1 + 42), _iso_at(_T1 + 99))),
    ("flush", {"machine_id": "m10", "now": _T1 + 43, "replies": []}),  # never reached: nothing sent
    ("flush", {"machine_id": "m99", "now": _T1 + 43, "replies": []}),  # unknown machine
    ("sql", _ob("r42", "m8", 1, "answer", _iso_at(_T1 + 42), _iso_at(_T1 + 50), attempts=1)),  # offline: expires
    ("sql", _ob("r43", "m9", 1, "answer", _iso_at(_T1 + 42), _iso_at(_T1 + 50), attempts=1)),  # no label: expires
    ("sql", _ob("r44", "m1", 50, "answer", _iso_at(_T1 + 42), _iso_at(_T1 + 50), attempts=1)),  # reachable, sent: waits
    ("sql", _ob("r45", "m1", 51, "answer", _iso_at(_T1 + 42), _iso_at(_T1 + 50))),  # never sent: expires
    ("sql", _ob("r46", "m1", 52, "answer", _iso_at(_T1 + 42), _iso_at(_T1 + 50), sending=_iso_at(_T1 + 75))),  # in flight
    ("sql", _ob("r47", "m1", 53, "answer", _iso_at(_T1 + 42), _iso_at(_T1 + 50), sending=_iso_at(_T1 + 69))),  # stale send
    ("sql", _ob("r48", "m1", 54, "answer", _iso_at(_T1 + 42), _iso_at(_T1 + 200))),  # not expired yet
    ("expire_sweep", {"now": _T1 + 100}),
    ("clearable", {"request_id": "r07", "now": _T1 + 100}),  # rejected
    ("clearable", {"request_id": "r44", "now": _T1 + 100}),  # queued, expired, not in flight
    ("clearable", {"request_id": "r46", "now": _T1 + 100}),  # in flight
    ("clearable", {"request_id": "r48", "now": _T1 + 100}),  # not expired
    ("clearable", {"request_id": "nope", "now": _T1 + 100}),
    ("clearable", {"request_id": "r09", "now": _T1 + 100}),  # done
    ("mark_seen", {"request_id": "r07"}),
    ("mark_seen", {"request_id": "r09"}),  # done: not seen
    ("outbox_rows", {}),
    # one step per remaining predicate, on a second machine with its own mirror
    ("pull", {"machine": _M2, "now": _T1 + 500, "export": _ex("d" * 32, 1, [
        _rq(1), _rq(2), _rq(3, "answered"), _rq(4, "dismissed")], full=True, ids=[1, 2, 3, 4])}),
    ("sql", _ob("o1", "m2", 1, "answer", _iso_at(_T1 + 501), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", _ob("o2", "m2", 3, "hide", _iso_at(_T1 + 499), _iso_at(_T1 + 9999), db="d" * 32)),  # older, inserted later
    ("sql", _ob("o3", "m2", 2, "answer", _iso_at(_T1 + 502), _iso_at(_T1 + 9999), db="c" * 32)),  # same qkey, other db
    ("sql", _ob("o4", "m2", 4, "hide", _iso_at(_T1 + 503), _iso_at(_T1 + 9999), db="d" * 32)),  # dismissed: hide goes
    ("sql", _ob("o5", "m2", 5, "answer", _iso_at(_T1 + 504), _iso_at(_T1 + 600), db="d" * 32)),  # expires right now
    ("sql", _ob("o6", "m2", 6, "answer", _iso_at(_T1 + 505), _iso_at(_T1 + 9999), db="d" * 32, attempts=4,
                qkey="00000006")),  # not in the mirror: rejected before any send
    ("sql", {"sql": "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at)"
                    " VALUES ('m2', ?, 6, '00000006', 'open', '{}', ?), ('m2', ?, 7, '00000007', 'open', '{}', ?)",
             "params": ["d" * 32, STORE_FIXED, "d" * 32, STORE_FIXED]}),
    ("sql", _ob("o7", "m2", 7, "answer", _iso_at(_T1 + 506), _iso_at(_T1 + 9999), db="d" * 32)),
    ("flush", {"machine_id": "m2", "now": _T1 + 600, "replies": [
        {"reply": {"outcome": "applied"}}, {"reply": {"outcome": "applied"}}, {"reply": {"outcome": "applied"}},
        {"stdout": "", "stderr": "lost", "code": 255}, {"reply": {"outcome": "rejected", "reason": ""}}]}),
    ("clearable", {"request_id": "00000000000000000000000000000002", "now": _T1 + 600}),  # failed
    ("clearable", {"request_id": "00000000000000000000000000000001", "now": _T1 + 50_000}),  # done, long expired
    ("clearable", {"request_id": "r48", "now": _T1 + 200}),  # expires exactly now
    ("sql", _ob("aaa-early", "m2", 9, "answer", _iso_at(_T1 - 100), _iso_at(_T1 - 50), db="d" * 32, state="rejected")),
    ("sql", _ob("s1", "m10", 2, "answer", _iso_at(_T1 + 650), _iso_at(_T1 + 700), attempts=1)),  # never reached
    ("sql", _ob("s2", "m2", 60, "answer", _iso_at(_T1 + 650), _iso_at(_T1 + 690), db="d" * 32,
                sending=_iso_at(_T1 + 670))),  # sent exactly SENDING_STALE ago: no longer in flight
    ("expire_sweep", {"now": _T1 + 700}),
    ("outbox_rows", {}),
    # a second worker settles the row during the send (#90 review M1), both ways; deliver on an
    # open question; the 30 s in-flight edge; a corrupt detail_json (Python raises before sending)
    ("sql", {"sql": "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at)"
                    " VALUES ('m2', ?, 10, '0000000a', 'open', '{}', ?), ('m2', ?, 11, '0000000b', 'open', '{}', ?),"
                    " ('m2', ?, 12, '0000000c', 'open', '{}', ?), ('m2', ?, 14, '0000000e', 'open', '{}', ?)",
             "params": ["d" * 32, STORE_FIXED] * 4}),
    ("sql", _ob("race1", "m2", 10, "answer", _iso_at(_T1 + 801), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", _ob("race2", "m2", 11, "answer", _iso_at(_T1 + 802), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", _ob("d1", "m2", 12, "deliver", _iso_at(_T1 + 803), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", {**_ob("c1", "m2", 14, "answer", _iso_at(_T1 + 804), _iso_at(_T1 + 9999), db="d" * 32)}),
    ("sql", {"sql": "UPDATE remote_outbox SET detail_json = '{broken' WHERE request_id = 'c1'", "params": []}),
    ("sql", _ob("s3", "m8", 13, "answer", _iso_at(_T1 + 800), _iso_at(_T1 + 850), sending=_iso_at(_T1 + 871))),
    ("sql", _ob("s4", "m8", 15, "answer", _iso_at(_T1 + 800), _iso_at(_T1 + 850), sending=_iso_at(_T1 + 870))),
    ("flush", {"machine_id": "m2", "now": _T1 + 900, "replies": [
        {"reply": {"outcome": "applied"}},  # o6, due again
        {"reply": {"outcome": "rejected", "reason": "late"},
         "before": "UPDATE remote_outbox SET state = 'done', result = 'applied' WHERE request_id = 'race1'"},
        {"reply": {"outcome": "applied"},
         "before": "UPDATE remote_outbox SET state = 'failed', result = 'expired' WHERE request_id = 'race2'"}]}),
    ("clearable", {"request_id": "s3", "now": _T1 + 900}),  # sent 29 s ago: in flight
    ("clearable", {"request_id": "s4", "now": _T1 + 900}),  # sent 30 s ago: not
    ("expire_sweep", {"now": _T1 + 900}),
    # time passes within one flush (#90 final pass M1): every _now() is 10 s after the one
    # before (``tick``). t1 is sent; t2 expires before its turn and is never sent; t3 is still
    # being sent (stamped late in the flush) when another worker's sweep runs, which leaves it
    # alone. u1's stored detail holds an escaped lone surrogate, sent back exactly as stored.
    ("sql", {"sql": "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at)"
                    " VALUES ('m2', ?, 20, '00000014', 'open', '{}', ?), ('m2', ?, 21, '00000015', 'open', '{}', ?),"
                    " ('m2', ?, 22, '00000016', 'open', '{}', ?), ('m2', ?, 23, '00000017', 'open', '{}', ?)",
             "params": ["d" * 32, STORE_FIXED] * 4}),
    ("sql", {"sql": "UPDATE remote_outbox SET state = 'seen' WHERE machine_id = 'm2' AND state = 'queued'",
             "params": []}),  # the rows above (c1 stays corrupt otherwise): out of the way
    ("sql", _ob("t1", "m2", 20, "answer", _iso_at(_T1 + 950), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", _ob("t2", "m2", 21, "answer", _iso_at(_T1 + 951), _iso_at(_T1 + 1025), db="d" * 32)),
    ("sql", _ob("t3", "m2", 22, "answer", _iso_at(_T1 + 952), _iso_at(_T1 + 1065), db="d" * 32)),
    ("flush", {"machine_id": "m2", "now": _T1 + 1000, "tick": 10, "replies": [
        {"reply": {"outcome": "applied"}},
        {"reply": {"outcome": "applied"}, "sweep": True}]}),
    ("sql", _ob("u1", "m2", 23, "answer", _iso_at(_T1 + 1100), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", {"sql": "UPDATE remote_outbox SET detail_json = ? WHERE request_id = 'u1'",
             "params": ['{"choices": [], "notes": {}, "text": "half \\ud83d pair"}']}),
    ("flush", {"machine_id": "m2", "now": _T1 + 1200, "replies": [{"reply": {"outcome": "applied"}}]}),
    # every other _now() of the outbox, with time passing (7 s per read): a due next try, a
    # stale mirror, a lost reply, a rejection, an old remote, the four status-query answers;
    # then the sweep, `d` and both enqueues
    ("sql", {"sql": "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at)"
                    " VALUES " + ", ".join(f"('m2', ?, {i}, '{i:08x}', 'open', '{{}}', ?)" for i in range(41, 50)),
             "params": ["d" * 32, STORE_FIXED] * 9}),
    ("sql", _ob("v1", "m2", 41, "answer", _iso_at(_T1 + 1301), _iso_at(_T1 + 9999), db="d" * 32,
                next_try=_iso_at(_T1 + 1290))),
    ("sql", _ob("v2", "m2", 42, "answer", _iso_at(_T1 + 1302), _iso_at(_T1 + 9999), db="c" * 32,
                next_try="")),  # an empty next try: no time read for it
    ("sql", _ob("v3", "m2", 43, "answer", _iso_at(_T1 + 1303), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", _ob("v4", "m2", 44, "answer", _iso_at(_T1 + 1304), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", _ob("v5", "m2", 45, "answer", _iso_at(_T1 + 1305), _iso_at(_T1 + 9999), db="d" * 32)),
    ("sql", _ob("v6", "m2", 46, "answer", _iso_at(_T1 + 1306), _iso_at(_T1 + 1000), db="d" * 32, attempts=1)),
    ("sql", _ob("v7", "m2", 47, "answer", _iso_at(_T1 + 1307), _iso_at(_T1 + 1000), db="d" * 32, attempts=1)),
    ("sql", _ob("v8", "m2", 48, "answer", _iso_at(_T1 + 1308), _iso_at(_T1 + 1000), db="d" * 32, attempts=1)),
    ("sql", _ob("v9", "m2", 49, "answer", _iso_at(_T1 + 1309), _iso_at(_T1 + 1000), db="d" * 32, attempts=1)),
    ("flush", {"machine_id": "m2", "now": _T1 + 1310, "tick": 7, "replies": [
        {"reply": {"outcome": "applied"}}, {"stdout": "", "stderr": "lost", "code": 255},
        {"reply": {"outcome": "rejected", "reason": "no"}}, {"code": 2, "stdout": ""},
        {"reply": {"outcome": "recorded", "state": "applied"}}, {"reply": {"outcome": "rejected"}},
        {"reply": {"outcome": "recorded", "reason": "closed"}}, {"reply": {"outcome": "expired"}}]}),
    ("sql", _ob("w1", "m9", 61, "answer", _iso_at(_T1 + 1400), _iso_at(_T1 + 1450), sending=_iso_at(_T1 + 1460))),
    ("sql", _ob("w2", "m9", 62, "answer", _iso_at(_T1 + 1400), _iso_at(_T1 + 1450), sending=_iso_at(_T1 + 1485))),
    ("sql", _ob("w3", "m9", 63, "answer", _iso_at(_T1 + 1400), _iso_at(_T1 + 1450))),
    ("expire_sweep", {"now": _T1 + 1480, "tick": 10}),  # w1 is out of flight at its read (1490), w2 not (1510)
    ("sql", _ob("w4", "m9", 64, "answer", _iso_at(_T1 + 1500), _iso_at(_T1 + 1550), sending=_iso_at(_T1 + 1525))),
    ("clearable", {"request_id": "w4", "now": _T1 + 1554, "tick": 1}),  # expired at 1554, stale at 1555: clearable
    ("sql", _ob("w5", "m9", 65, "answer", _iso_at(_T1 + 1500), _iso_at(_T1 + 1555))),
    ("clearable", {"request_id": "w5", "now": _T1 + 1554, "tick": 1}),  # one read: not expired yet
    # an unsafe target on the send path: call() refuses it inside _sending, the row fails
    ("sql", {"sql": "INSERT INTO remote_machines (machine_id, label, target, last_error, last_ok_at)"
                    " VALUES ('m11', 'eleven', 'x;y', NULL, ?)", "params": [_iso_at(_T1)]}),
    ("sql", {"sql": "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at)"
                    " VALUES ('m11', ?, 1, '00000001', 'open', '{}', ?), ('m11', ?, 2, '00000002', 'open', '{}', ?)",
             "params": [_DBX, STORE_FIXED] * 2}),
    ("sql", _ob("x1", "m11", 1, "answer", _iso_at(_T1 + 1560), _iso_at(_T1 + 9999))),
    ("sql", _ob("x2", "m11", 2, "answer", _iso_at(_T1 + 1561), _iso_at(_T1 + 9999))),
    ("flush", {"machine_id": "m11", "now": _T1 + 1570, "tick": 3, "replies": []}),
    ("enqueue", {"machine_id": "m2", "data": {"id": 20, "qkey": "00000014", "_db_id": "d" * 32}, "action": "answer",
                 "detail": None, "now": _T1 + 1600, "tick": 5}),
    ("enqueue_alert_ack", {"machine_id": "m1", "data": {"id": 2, "akey": "00000002aa", "_db_id": _DBX},
                           "now": _T1 + 1700, "tick": 5}),
    ("outbox_rows", {}),
]
# What validate_export makes of exports (the rows it keeps, cleaned, or the error it raises).
_VROW = {"id": 1, "qkey": "0123abcd", "group": 1, "status": "open", "question": "Q?"}
STORE_EXPORT_VECTORS = [
    _ex(_DBA, 1, [_VROW]),
    _ex(_DBA, 1, [{**_VROW, "question": "a b\x07c‮d\x85e" + "x" * 5000, "header": "h" * 99,
                   "tab_label": "t ", "options": ["a", {"label": "b", "description": 5, "preview": "p"},
                                                      {"label": "c\x1b", "description": "d" * 600}],
                   "multi_select": 1, "hidden": "", "agent_gone": [0], "delivery_attempts": True}]),
    _ex(_DBA, 1, [{**_VROW, "id": True, "group": False, "delivery_attempts": -1},
                  {**_VROW, "id": 2, "delivery_attempts": 10_000}, {**_VROW, "id": 3, "delivery_attempts": 9999}]),
    _ex(_DBA, 1, [{**_VROW, "options": ["o"] * 10}, {**_VROW, "id": 2, "options": [1]},
                  {**_VROW, "id": 3, "options": "abc"}, {**_VROW, "id": 4, "options": 0},
                  {**_VROW, "id": 5, "options": [{"label": 3}]}, {**_VROW, "id": 6, "status": "gone"},
                  {**_VROW, "id": 7, "question": ""}, {**_VROW, "id": 8, "qkey": "abc"},
                  {**_VROW, "id": 9, "qkey": "A" * 8}, {**_VROW, "id": 10, "header": 5}, [1],
                  {**_VROW, "id": "11"}, {**_VROW, "id": 12, "group": None}, {**_VROW, "id": 13, "qkey": "f" * 65}]),
    _ex(_DBA, 1, [], alerts=[_ra(1, kind="other", severity="loud", title="", message="m\nx"), "x",
                            _ra(2, akey="ZZZZZZZZ"), _ra(3, status="bogus"), {**_ra(4), "id": "4"},
                            _ra(5, tab_label="t" * 200, pane_id="p" * 100)]),
    _ex(_DBA, 1, [], alerts=[_ra(1, title=5)]),  # a non-string title: the whole export is refused
    _ex(_DBA, 1, [], alerts="many"),
    _ex(_DBA, 1, [], full=True),
    _ex(_DBA, 1, [], full=True, ids=[1, "2"]),
    _ex(_DBA, 1, [], full=1, ids=[1, True]),
    _ex(_DBA, 1, [], gone=[1, 2.5]),
    _ex(_DBA, 1, [], gone=0, ids="whatever"),
    _ex(_DBA, True, []),
    _ex(_DBA, 1.0, []),
    _ex(_DBA, 1, "rows"),
    _ex(_DBA, 1, [0] * 1001),  # more than 1000 rows: refused before any is read
    {**_ex(_DBA, 1, []), "format": 1.0, "version": None},
    {**_ex(_DBA, 1, []), "format": "1"},
    {**_ex(_DBA, 1, []), "format": True},  # True == 1 in Python
    {**_ex(_DBA, 1, []), "format": None, "version": 12},
    {k: v for k, v in _ex(_DBA, 1, []).items() if k != "format"},
    [1, 2],
    _ex(_DBA, 1, [{**_VROW, "question": "a\x00b\x08c\x0bd\x0ce\x1ff\x7fg\x9fh\u200ei\u200fj\u2066k\u2069l\x09m\x0an"}]),
    _ex(_DBA, 1, [{**_VROW, "options": [{"description": "no label"}]}]),
    _ex(_DBA, 1, [], alerts=[_ra(1, akey="a" * 64), _ra(2, akey="a" * 65), _ra(3, akey="a" * 7)]),
    _ex(_DBA, 1, [], full=None, gone=None),
    _ex(_DBA, 1, [], full=[], gone=[]),
    _ex(_DBA, 1, [], alerts=[0] * 1000 + [_ra(1)]),  # only the first 1000 entries are looked at
    _ex(_DBA, 1, [], gone=list(range(1001))),
    _ex(_DBA, 1, [{**_VROW, "qkey": "ghijklmn"}], alerts=[_ra(1, akey="ghijklmn"), _ra(2, status="x")]),  # non-hex; unknown status
    _ex("ghijklmn" * 4, 1, []),  # a non-hex db_id
    _ex(_DBA, 1, [{**_VROW, "options": ["l" * 201, {"label": "m" * 201}]}]),  # labels cut to 200
    {**_ex(_DBA, 1, []), "version": "v" * 21},  # cut to 20
    _ex(_DBA, 1, [{**_VROW, "qkey": "gggggggg"}], alerts=[_ra(1, akey="gabcdefa")]),  # g is not hex
    _ex("g" * 32, 1, []),
    _ex(_DBA, 1, [{**_VROW, **{k: ("f" if k == "qkey" else "z") * (cap + 1) for k, cap in (
        ("question", 4000), ("header", 60), ("tab_label", 120), ("pane_id", 64), ("asked_by", 120), ("title", 120),
        ("answer", 4000), ("delivery_error", 300), ("created_at", 40), ("answered_at", 40), ("delivered_at", 40),
        ("verified_at", 40), ("acknowledged_at", 40), ("qkey", 64))}}]),  # every field one over its cap
    _ex(_DBA, 1, [{**_VROW, "options": [{"label": "o", "preview": "p" * 4001, "description": "d" * 501}]}],
        alerts=[_ra(1, message="m" * 2001, tab_label="t" * 121, pane_id="p" * 81, asked_by="a" * 121, created_at="c" * 41)]),
]
# What normalize_options makes of malformed options (the MCP schema allows strings only; a stored
# row or another writer may hold anything): its result, or the exception it raises.
STORE_OPTIONS = [["a", {"label": "b", "description": "", "preview": "p"}],
                 [{"label": 5, "description": False, "preview": 0}],
                 [{"label": True, "description": [], "preview": {}}], [{"label": None}],
                 [{"label": 1.5, "description": 5, "preview": [1]}], [{"label": 1e20, "description": 0.0}],
                 [{"label": "x", "description": True, "preview": "y"}],
                 [{"description": "no label"}], [5], [None], [["a"]], [], None, "", "ab", {"k": 1, "j": 2},
                 {}, 0, 7, True]
STORE_TEXTS = ["Which REGION?", "  a\tb\n\nc  ", "Proceed — with the next step?!", "über_ALLES 3.5%",
               "İstanbul ǅemal ß", "日本語のテスト？", "tab\x1csep\x85nel", "", "!!!", "emoji 🙂 here",
               "x" * 70, "Deploy\tplan:\x07 ok", "\u2028line\u2029para", "ﬁne ﬂow"]


def _store_value(value):
    """A Python result as JSON: questions by every field (times masked), the rest as is."""
    import dataclasses
    import re

    from herdr_inbox.store import Question

    if isinstance(value, Question):
        value = dataclasses.asdict(value)
    if isinstance(value, dict):
        return {k: _store_value(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_store_value(v) for v in value]
    if isinstance(value, str) and value != STORE_FIXED and re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+00:00", value):
        return "<time>"
    if isinstance(value, str) and re.fullmatch(r"sending:\d{4}-\d{2}-\d{2}T[\d:]+\+00:00:[0-9a-f]{8}", value):
        return "sending:<time>:<token>"
    return value


def _store_tables(path: Path, tables=("questions", "question_ext", "question_groups")) -> dict:
    import sqlite3

    db = sqlite3.connect(path)
    try:
        out = {}
        for table in tables:
            cur = db.execute(f"SELECT * FROM {table} ORDER BY 1")
            names = [d[0] for d in cur.description]
            out[table] = [_store_value(dict(zip(names, row))) for row in cur.fetchall()]
        return out
    finally:
        db.close()


def capture_store(out: Path, work: Path) -> list[Path]:
    """The question reads and writes of store.Store over STORE_OPS (see the module docstring)."""
    from herdr_inbox import store as st

    path = work / "store-questions.db"
    steps = []
    with st.Store(path, migrate=True) as s:
        for op, args in STORE_OPS:
            result = s.db.execute(args["sql"], args["params"]) and None if op == "sql" else getattr(s, op)(**args)
            steps.append({"op": op, "args": args, "result": _store_value(result), "tables": _store_tables(path)})
    texts = [{"text": t, "normalize": st.normalize(t), "clean_title": st.clean_title(t)} for t in STORE_TEXTS]
    options = []
    for value in STORE_OPTIONS:
        try:
            options.append({"input": value, "output": st.normalize_options(value)})
        except Exception as e:  # noqa: BLE001
            options.append({"input": value, "error": type(e).__name__})
    doc = {"source": "store.Store over STORE_OPS (tools/capture_goldens.py --only store)", "steps": steps,
           "texts": texts, "options": options}
    target = out / "store-questions.json"
    target.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n")
    return [target, capture_store_delivery(out, work), capture_store_alerts(out, work), capture_store_mirror(out, work), capture_store_outbox(out, work)]


def capture_store_alerts(out: Path, work: Path) -> Path:
    """alerts.py's store side over STORE_ALERT_OPS (herdr stubbed, its effects recorded)."""
    import dataclasses
    import itertools

    from herdr_inbox import alerts, herdr, service, store as st, transcript

    path = work / "store-alerts.db"
    effects: list = []
    env = {"server": "/srv/a", "agents": None, "labels": {}, "t": 0.0}
    stubs = {
        (herdr, "server_id"): lambda: env["server"],
        (herdr, "notify"): lambda title, body=None, *a, **k: effects.append(["notify", title, body]),
        (herdr, "call"): lambda *args, **k: (({"agents": env["agents"]}, None) if args[:2] == ("agent", "list")
                                             and env["agents"] is not None else (None, "fail")),
        (herdr, "tab_label"): lambda tab: env["labels"].get(tab),
        (service, "update_markers"): lambda store, pane: effects.append(["markers", pane]),
        (service, "later"): lambda fn: fn(),
        (alerts.time, "monotonic"): lambda: env["t"],
    }
    saved = {k: getattr(*k) for k in stubs}
    played = []
    for (mod, name), fn in stubs.items():
        setattr(mod, name, fn)
    real_chain = transcript.chain
    transcript.chain = lambda session, path_: played.append(session) or [session]
    alerts._unblocked.clear()
    steps = []
    try:
        with st.Store(path, migrate=True) as s:
            for op, args in STORE_ALERT_OPS:
                effects.clear()
                played.clear()
                caller = service.Caller(**args["caller"], cwd=None) if "caller" in args else None
                env["server"] = args.get("server", "/srv/a")
                if op == "sql":
                    s.db.execute(args["sql"], args["params"])
                    result = None
                elif op == "raise_alert":
                    result = alerts.raise_alert(s, caller, args["title"], args["message"], args["severity"],
                                                args["asked_by"])
                    result = {k: result[k] for k in ("id", "status") if k in result}
                elif op in ("withdraw",):
                    result = {k: v for k, v in alerts.withdraw(s, caller, args["aid"]).items() if k != "message"}
                elif op in ("mine", "acknowledged_unreported"):
                    result = getattr(alerts, op)(s, caller)
                elif op in ("get", "acknowledge"):
                    result = getattr(alerts, op)(s, args["aid"])
                elif op == "open_alerts":
                    result = alerts.open_alerts(s)
                elif op in ("scan", "scan_failed"):
                    env.update(agents=args.get("agents"), labels=args.get("labels", {}), t=args.get("t", 0.0))
                    result = alerts.scan_blocked(s)
                elif op == "acknowledge_answers":
                    result = s.acknowledge(args["qids"])
                elif op == "sessions_in_play":
                    service.adopt_continuations(s)
                    result = sorted(played)
                else:
                    result = getattr(s, op)(**args)
                if isinstance(result, list):
                    result = [dataclasses.asdict(a) if dataclasses.is_dataclass(a) else a for a in result]
                elif dataclasses.is_dataclass(result):
                    result = dataclasses.asdict(result)
                steps.append({"op": op, "args": args, "result": _store_value(result), "effects": list(effects),
                              "tables": _store_tables(path, ("alerts", "blocked_panes"))})
    finally:
        for (mod, name), fn in saved.items():
            setattr(mod, name, fn)
        transcript.chain = real_chain
        alerts._unblocked.clear()
    del itertools
    doc = {"source": "alerts.py over STORE_ALERT_OPS (tools/capture_goldens.py --only store)", "steps": steps}
    target = out / "store-alerts.json"
    target.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n")
    return target


def _remote_run(remote, env: dict, effects: list):
    """subprocess.run as remote.py's ssh() and call() use it, scripted per step: the probe and
    `herdr plugin list` from the step (else _PROBE_OUT, _PLUGINS_OUT), `inbox export` from the
    step's code/export/stdout/stderr, each `inbox apply` from the next of env["replies"]. Every
    call is recorded (target, command, stdin)."""
    import subprocess as sp

    def run(argv, input=None, capture_output=True, text=True, timeout=None):
        target, command, s = argv[-2], argv[-1], env["step"]
        if input is None:
            effects.append(["ssh", target, command])
            if command == remote.PROBE:
                return sp.CompletedProcess(argv, 0, s.get("probe", _PROBE_OUT), "")
            return sp.CompletedProcess(argv, 0, s.get("plugins", _PLUGINS_OUT), "")
        effects.append(["call", target, command, input])
        if command.endswith(" inbox export"):
            stdout = json.dumps(s["export"]) if "export" in s else s.get("stdout", "")
            return sp.CompletedProcess(argv, s.get("code", 0), stdout, s.get("stderr", ""))
        r = env["replies"].pop(0) if env.get("replies") else {"code": 255, "stderr": "ssh: no reply"}
        stdout = json.dumps(r["reply"]) if "reply" in r else r.get("stdout", "")
        return sp.CompletedProcess(argv, r.get("code", 0), stdout, r.get("stderr", ""))

    return run


def capture_store_mirror(out: Path, work: Path) -> Path:
    """remote.py's mirror over STORE_MIRROR_OPS, and validate_export over STORE_EXPORT_VECTORS."""
    import sqlite3
    import subprocess as sp
    from datetime import datetime, timezone

    from herdr_inbox import remote, store as st

    path = work / "store-mirror.db"
    env = {"now": _T0, "step": {}}
    effects: list = []

    run = _remote_run(remote, env, effects)
    stubs = {"_now": lambda: datetime.fromtimestamp(env["now"], tz=timezone.utc),
             "_notify": lambda title, body: effects.append(["notify", title, body])}
    saved = {k: getattr(remote, k) for k in stubs}
    for k, fn in stubs.items():
        setattr(remote, k, fn)
    real_run, remote.subprocess.run = remote.subprocess.run, run

    def tables():
        db = sqlite3.connect(path)
        try:
            out = {}
            for table in ("remote_machines", "remote_questions", "remote_alerts", "remote_outbox"):
                cur = db.execute(f"SELECT * FROM {table} ORDER BY 1, 2")
                names = [d[0] for d in cur.description]
                out[table] = [dict(zip(names, row)) for row in cur.fetchall()]
            return out
        finally:
            db.close()

    steps = []
    try:
        with st.Store(path, migrate=True) as s:
            for op, args in STORE_MIRROR_OPS:
                effects.clear()
                env["step"] = args
                env["now"] = args.get("now", env["now"])
                if op == "sql":
                    s.db.execute(args["sql"], args["params"])
                    result = None
                elif op == "pull":
                    result = remote.pull(s, args["machine"])
                elif op == "open_alerts":
                    result = [[m, a] for m, a in remote.open_alerts(s)]
                elif op == "alert_by_key":
                    got = remote.alert_by_key(s, args["key"])
                    result = list(got) if got else None
                elif op == "open_groups":
                    result = [[k, m, qs] for k, m, qs in remote.open_groups(s)]
                elif op == "group_by_key":
                    got = remote.group_by_key(s, args["key"])
                    result = list(got) if got else None
                elif op == "problem_rows":
                    result = remote.problem_rows(s)
                elif op == "clear_problem":
                    result = remote.clear_problem(s, args["machine_id"], args["remote_id"])
                elif op == "notify_problems":
                    result = remote.notify_problems(s, args["machine_id"], args["label"])
                steps.append({"op": op, "args": args, "result": result, "effects": list(effects), "tables": tables()})
    finally:
        for k, fn in saved.items():
            setattr(remote, k, fn)
        remote.subprocess.run = real_run
    vectors = []
    for v in STORE_EXPORT_VECTORS:
        raw = json.dumps(v)
        try:
            vectors.append({"raw": raw, "result": remote.validate_export(raw)})
        except (ValueError, PermissionError) as e:
            vectors.append({"raw": raw, "error": type(e).__name__, "message": str(e)})
    doc = {"source": "remote.py over STORE_MIRROR_OPS and STORE_EXPORT_VECTORS (tools/capture_goldens.py --only store)",
           "steps": steps, "vectors": vectors}
    target = out / "store-mirror.json"
    target.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n")
    return target


def capture_store_outbox(out: Path, work: Path) -> Path:
    """remote.py's outbox over STORE_OUTBOX_OPS (ssh scripted at subprocess.run)."""
    import sqlite3
    from datetime import datetime, timezone

    from herdr_inbox import remote, store as st

    path = work / "store-outbox.db"
    env = {"now": _T1, "step": {}, "replies": []}
    effects: list = []
    inner = _remote_run(remote, env, effects)

    def run(argv, input=None, **kw):
        if input is not None and argv[-1].endswith(" inbox apply"):
            rid = json.loads(input)["request_id"]
            db = sqlite3.connect(path)
            try:
                row = db.execute("SELECT sending_at FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()
            finally:
                db.close()
            effects.append(["sending", rid, row[0] if row else None])
            nxt = env["replies"][0] if env.get("replies") else {}
            if nxt.get("sweep"):  # another worker's expiry sweep runs while this send is under way,
                tick, env["tick"] = env["tick"], 0  # at the time now, on a clock of its own
                notify, remote._notify = remote._notify, lambda title, body: effects.append(
                    ["notify-other", title, body])  # its notices, not this flush's
                try:
                    with st.Store(path, maintenance=False) as other:
                        remote.expire_sweep(other)
                finally:
                    env["tick"], remote._notify = tick, notify
            if nxt.get("before"):  # another worker settles the row while this send is under way
                db = sqlite3.connect(path, isolation_level=None)
                try:
                    db.execute(nxt["before"])
                finally:
                    db.close()
        return inner(argv, input=input, **kw)

    count = {"n": 0}

    class _U:
        def __init__(self):
            count["n"] += 1
            self.hex = f"{count['n']:032x}"

    def _now():  # the step's ``now``, then ``tick`` seconds later at every read
        at = env["now"]
        env["now"] += env.get("tick", 0)
        return datetime.fromtimestamp(at, tz=timezone.utc)

    stubs = {"_now": _now, "_notify": lambda title, body: effects.append(["notify", title, body])}
    saved = {k: getattr(remote, k) for k in stubs}
    for k, fn in stubs.items():
        setattr(remote, k, fn)
    real_run, remote.subprocess.run = remote.subprocess.run, run
    real_uuid, remote._uuid.uuid4 = remote._uuid.uuid4, _U

    def tables():
        db = sqlite3.connect(path)
        try:
            out = {}
            for table in ("remote_outbox", "remote_alerts"):
                cur = db.execute(f"SELECT * FROM {table} ORDER BY 1, 2")
                names = [d[0] for d in cur.description]
                out[table] = [dict(zip(names, row)) for row in cur.fetchall()]
            return out
        finally:
            db.close()

    steps = []
    try:
        with st.Store(path, migrate=True) as s:
            for op, args in STORE_OUTBOX_OPS:
                effects.clear()
                env["step"] = args
                env["now"] = args.get("now", env["now"])
                env["tick"] = args.get("tick", 0)
                env["replies"] = list(args.get("replies", []))
                try:
                    if op == "sql":
                        s.db.execute(args["sql"], args["params"])
                        result = None
                    elif op == "pull":
                        result = remote.pull(s, args["machine"])
                    elif op == "enqueue":
                        result = remote.enqueue(s, args["machine_id"], args["data"], args["action"], args["detail"])
                    elif op == "enqueue_alert_ack":
                        result = remote.enqueue_alert_ack(s, args["machine_id"], args["data"])
                    elif op == "flush":
                        result = remote.flush(s, {"id": args["machine_id"]})
                    elif op == "expire_sweep":
                        result = remote.expire_sweep(s)
                    elif op == "clearable":
                        result = remote.clearable(s, args["request_id"])
                    elif op == "mark_seen":
                        result = remote.mark_seen(s, args["request_id"])
                    elif op == "outbox_rows":
                        result = remote.outbox_rows(s)
                except Exception as e:  # noqa: BLE001 - what Python raises is part of the behaviour
                    result = {"error": type(e).__name__, "message": str(e)}
                steps.append({"op": op, "args": args, "result": result, "effects": list(effects), "tables": tables()})
    finally:
        for k, fn in saved.items():
            setattr(remote, k, fn)
        remote.subprocess.run = real_run
        remote._uuid.uuid4 = real_uuid
    doc = {"source": "remote.py's outbox over STORE_OUTBOX_OPS (tools/capture_goldens.py --only store)", "steps": steps}
    target = out / "store-outbox.json"
    target.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n")
    return target


def capture_store_delivery(out: Path, work: Path) -> Path:
    """store.Store's delivery-state reads and writes over STORE_DELIVERY_OPS."""
    from datetime import timedelta

    from herdr_inbox import store as st

    path = work / "store-delivery.db"
    tables = ("questions", "question_ext", "sessions")
    steps, claims = [], {}
    with st.Store(path, migrate=True) as s:
        for op, args in STORE_DELIVERY_OPS:
            call = {}
            for k, v in args.items():
                if isinstance(v, str) and v.startswith("$claim:"):
                    v = claims[int(v.split(":")[1])]
                if k.endswith("_s"):
                    k, v = k[:-2], timedelta(seconds=v)
                call[k] = tuple(v) if k == "prefixes" else v
            if op == "sql":
                s.db.execute(args["sql"], args["params"])
                result = None
            else:
                result = getattr(s, op)(**call)
            if op == "claim_delivery":
                claims[args["qid"]] = result
            steps.append({"op": op, "args": args, "result": _store_value(result),
                          "tables": _store_tables(path, tables)})
    doc = {"source": "store.Store over STORE_DELIVERY_OPS (tools/capture_goldens.py --only store)",
           "steps": steps}
    target = out / "store-delivery.json"
    target.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n")
    return target


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--out", type=Path, default=FIXTURES,
                   help="write OUT/screens and OUT/decisions.json (default: %(default)s)")
    p.add_argument("--only", choices=("screens", "decisions", "schema", "landed", "store", "herdr"),
                   help="one of the sets")
    p.add_argument("--golden-tests", action="store_true",
                   help="decisions: run only the tests the committed decisions.json names (the check; any "
                        "other test drives no traced decision, so the table comes out the same)")
    args = p.parse_args(argv)
    work = Path(tempfile.mkdtemp(prefix="inbox-goldens-"))
    env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
    try:
        isolate(work)
        written = []
        if args.only in (None, "decisions"):
            tests = None
            if args.golden_tests:
                rows = json.loads((FIXTURES / "decisions.json").read_text())["rows"]
                tests = sorted({t for r in rows for t in r["tests"]})
            written += capture_decisions(args.out, work, env, tests)
        if args.only in (None, "schema"):
            written += capture_schema(args.out)
        if args.only in (None, "landed"):
            written += capture_landed(args.out)
        if args.only in (None, "herdr"):
            written.append(capture_herdr_screens(args.out))
        if args.only in (None, "store"):
            written += capture_store(args.out, work)
        if args.only in (None, "screens"):
            shutil.rmtree(args.out / "screens", ignore_errors=True)
            written += capture_screens(args.out, work)
        size = sum(f.stat().st_size for f in written)
        print(f"{len(written)} files, {size // 1024} KiB under {args.out}")
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
