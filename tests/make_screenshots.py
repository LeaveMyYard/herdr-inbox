"""Regenerate docs/screenshots/*.png from neutral demo data.

Not collected by a plain `pytest` run. Usage (needs `rsvg-convert`):

    uv run pytest tests/make_screenshots.py -q

Each shot renders the popup headlessly (Textual SVG export) and converts it to PNG.
"""

import subprocess
from pathlib import Path

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp

OUT = Path(__file__).resolve().parent.parent / "docs" / "screenshots"
BILLING = Caller(pane_id="w1:p1", tab_id="w1:t1", tab_label="Billing API", session_id="s1", cwd="/r")
SEARCH = Caller(pane_id="w1:p2", tab_id="w1:t2", tab_label="Search refactor", session_id="s2", cwd="/r")
DOCS = Caller(pane_id="w1:p3", tab_id="w1:t3", tab_label="Docs site", session_id="s3", cwd="/r")

DB = [{"label": "Postgres", "description": "keeps the current ORM and migrations"},
      {"label": "SQLite", "description": "single file, no server"}, {"label": "DuckDB"}]


def populate(store):
    service.ask(store, SEARCH, "Merge PR #42 with a merge commit or squash?", options=["merge commit", "squash"],
                header="Merge mode", asked_by="reviewer")                                              # Q1
    service.ask_group(store, BILLING, [
        {"question": "Which database should the invoice store use?", "header": "DB choice", "options": DB},
        {"question": "Which caches should the API warm on deploy?", "header": "Cache", "multi_select": True,
         "options": ["redis", "in-process", "CDN"]}], context="Invoice rows ~1M/day", asked_by="planner")  # Q2, Q3
    service.ask(store, DOCS, "Publish the v2 docs now or after the release?", options=["now", "after"],
                header="Rollout")                                                                       # Q4


def shot(app, name, size=(110, 30)):
    svg = OUT / f"{name}.svg"
    svg.write_text(app.export_screenshot(title="herdr inbox"))
    subprocess.run(["rsvg-convert", "-w", "1400", "-o", str(OUT / f"{name}.png"), str(svg)], check=True)
    svg.unlink()


async def test_shots(store, fake_herdr):
    size = (125, 30)
    # empty
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await pilot.pause()
        shot(app, "empty")
    populate(store)
    # submit tab: three questions drafted, one still blank
    for qid, pick in ((1, "squash"), (2, "SQLite"), (3, "redis")):
        service.save_draft(store, qid, {"choices": [pick]})
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await pilot.pause()
        for _ in range(4):
            await pilot.press("right")
        await pilot.pause()
        shot(app, "submit")
    # tabs: Q1 was answered but could not be delivered -> a warning tab
    for qid in (2, 3):
        service.save_draft(store, qid, None)
    store.set_answer(1, "squash", {"choices": ["squash"]})
    store.record_delivery(1, "no live agent in the pane")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await pilot.pause()
        await pilot.press("down")
        shot(app, "tabs")


async def test_single_and_preview(fake_herdr, tmp_path, monkeypatch):
    from herdr_inbox.store import Store

    size = (125, 30)
    monkeypatch.setenv("HERDR_INBOX_DB", str(tmp_path / "one.db"))
    store = Store()
    service.ask(store, BILLING, "Which database should the invoice store use?", options=DB, header="DB choice",
                context="Invoice rows ~1M/day", asked_by="planner")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await pilot.pause()
        await pilot.press("down")
        shot(app, "single")
    store.close()
    monkeypatch.setenv("HERDR_INBOX_DB", str(tmp_path / "preview.db"))
    store = Store()
    opts = [{"label": "Small", "description": "2 workers", "preview": '[worker]\nthreads = 2\nmemory = "1Gi"\ncpu = "500m"'},
            {"label": "Large", "description": "8 workers", "preview": '[worker]\nthreads = 8\nmemory = "4Gi"\ncpu = "2"'},
            {"label": "Custom"}]
    service.ask(store, BILLING, "Which worker profile for the invoice exporter?", options=opts, header="Profile")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        await pilot.pause()
        shot(app, "preview")
    store.close()
