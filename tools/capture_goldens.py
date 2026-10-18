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
                for ts in ("2026-10-17T10:00:05.000Z", "2026-10-17T09:00:00.000Z", None):
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


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--out", type=Path, default=FIXTURES,
                   help="write OUT/screens and OUT/decisions.json (default: %(default)s)")
    p.add_argument("--only", choices=("screens", "decisions", "schema", "landed"), help="one of the sets")
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
