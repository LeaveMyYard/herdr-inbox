"""Pilot tests for the popup: every open question as a tab, Claude-Code style."""

from textual.widgets import Input, Static

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.store import Store
from herdr_inbox.tui import CHAT, OTHER, GroupListView, InboxApp, QuestionView, ReviewView, tab_order

A = Caller(pane_id="w1:p1", tab_id="w1:t1", tab_label="tab-A", session_id="aaaaaaaa-0000-4000-8000-00000000000a", cwd="/r")
B = Caller(pane_id="w1:p2", tab_id="w1:t2", tab_label="tab-B", session_id="aaaaaaaa-0000-4000-8000-00000000000a", cwd="/r")

DB = [{"label": "Postgres", "description": "keeps the current ORM"}, {"label": "SQLite", "description": "single file"},
      {"label": "DuckDB"}]


def view(app) -> QuestionView:
    return app.query_one(QuestionView)


def text_of(widget) -> str:
    return str(widget.render())


def chips(app) -> list[str]:
    return [" ".join(text_of(c).replace("\u00a0", " ").split()) for c in app.query("#chips > Static")]


def prompts(fake):
    return [c[3] for c in fake.calls_of("agent", "prompt")]


async def opened(app, pilot):
    await pilot.pause()
    return app


# -- tab order --------------------------------------------------------------------

def test_tab_order_focused_pane_first_groups_adjacent(store, fake_herdr):
    service.ask(store, B, "B old?")                                                    # 1
    service.ask_group(store, A, [{"question": "A one?"}, {"question": "A two?"}])    # 2, 3
    service.ask(store, B, "B new?")                                                    # 4
    service.ask(store, A, "A later?")                                                  # 5
    qs = store.list()
    assert tab_order(qs, "w1:p1") == [2, 3, 5, 1, 4]
    assert tab_order(qs, None) == [1, 2, 3, 4, 5]


async def test_group_list_rows_and_order(store, fake_herdr):
    """Was 'tabs show agent boundaries': each agent's call is now its own list row."""
    service.ask(store, B, "B one?", header="Deploy")
    service.ask_group(store, A, [{"question": "A one?", "header": "DB"}, {"question": "A two?"}], asked_by="planner")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await opened(app, pilot)
        assert app.mode == "list" and app.entries == [("group", 2), ("group", 1)]  # focused pane first
        lv = app.query_one(GroupListView)
        assert lv.cursor == ("group", 2)
        first, second = (lv.row_text(1, ("group", 2)).replace("\u00a0", " "),
                          lv.row_text(2, ("group", 1)).replace("\u00a0", " "))
        # no title given: the name falls back to the first header (see test_group_title.py)
        assert "DB · 2 questions" in first and "tab-A · planner · DB · A two?" in first
        assert first.startswith("[$inbox-suggestion]❯[/] ") and "\n" in first  # multiline rows
        assert "Deploy · 1 question" in second and second.endswith("[dim]tab-B[/dim]")  # no repeated chip
        assert text_of(lv.query_one(".hint")) == "Enter to open · ↑/↓ to navigate · g go to agent · d dismiss · Esc to cancel"
        await pilot.press("enter")
        assert app.mode == "group" and view(app).q.id == 2
        assert [c.strip() for c in chips(app)] == ["│ tab-A", "● DB", "● Q3", "Submit"]  # this group only
        assert "tab-A" in text_of(view(app).query_one(".meta"))


# -- one open question: answers send at once ------------------------------------------

async def test_single_question_layout_and_keys(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB, header="DB choice")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        v = view(app)
        assert chips(app) == [] and "DB choice" in text_of(v.query_one(".chip"))
        assert [k for k, _ in v.rows] == ["opt", "opt", "opt", "other", "chat"]
        assert OTHER in v.row_text(3) and CHAT in v.row_text(4)
        await pilot.press("down", "j")
        assert v.cursor == 2
        await pilot.press("k", "4")
        assert v.cursor == 3
        await pilot.press("1")
        assert v.cursor == 0
        assert text_of(v.query_one(".hint")) == ("Enter to select · ↑/↓ to navigate · press n to add notes · "
                                                 "g go to agent · d dismiss · Esc to cancel")


async def test_single_pick_sends_immediately_and_closes(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("2", "n", *"no server", "enter", "enter")  # note, then pick SQLite
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert not app.is_running  # nothing left: the popup is gone, like Claude Code's prompt
    q = store.get(1)
    assert q.status == "answered" and q.answer == "SQLite (note: no server)"
    assert prompts(fake_herdr) == ["[inbox] Q1 answered — Which DB?: SQLite (note: no server)"]


async def test_type_something_and_multi_select(store, fake_herdr):
    service.ask(store, A, "Which DBs?", options=DB, multi_select=True)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        v = view(app)
        assert [k for k, _ in v.rows][-2:] == ["other", "submit"]
        await pilot.press("enter", "3", "space")  # Postgres, DuckDB
        await pilot.press("4", "enter", *"also redis", "enter")  # added to the picks
        await pilot.press("down", "enter")  # Submit
        await app.workers.wait_for_complete()
    assert store.get(1).answer == "Postgres, DuckDB, also redis"


async def test_chat_about_this_and_escape(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("n", *"dq", "escape")  # Esc cancels the note box only
        assert app.is_running and view(app).notes == {}
        await pilot.press("5", "enter")  # Chat about this
        await fake_herdr.went_to_agent(app)
    assert fake_herdr.calls_of("agent", "focus") == [["agent", "focus", "w1:p1"]]
    assert store.get(1).status == "open" and prompts(fake_herdr) == []
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("escape")  # closes without sending
        await pilot.pause()
        assert not app.is_running
    assert store.get(1).status == "open"


async def test_old_string_only_options(store, fake_herdr):
    store.db.execute("INSERT INTO questions (created_at, question, norm, options, pane_id, session_id, status)"
                     " VALUES ('2026-10-14T00:00:00+00:00', 'Legacy?', 'legacy', '[\"yes\", \"no\"]', 'w1:p1', 'aaaaaaaa-0000-4000-8000-00000000000a', 'open')")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("2", "enter")
        await app.workers.wait_for_complete()
    assert store.get(1).answer == "no"


async def test_preview_layout(store, fake_herdr):
    service.ask(store, A, "Which config?", options=[
        {"label": "Small", "preview": "workers = 2\nmemory = 1Gi"}, {"label": "Large", "description": "x"}])
    app = InboxApp(store)
    async with app.run_test(size=(100, 30)) as pilot:
        await opened(app, pilot)
        preview = app.query_one("#preview", Static)
        assert "workers = 2" in text_of(preview)
        await pilot.press("down")
        assert text_of(preview) == "No preview available"
        assert app.query_one(".split > .choices").region.x < app.query_one(".preview-box").region.x


async def test_empty_state_closes_on_any_key(store, fake_herdr):
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        assert text_of(app.query_one("#empty")) == "No open questions" and app.query_one("#empty").display
        await pilot.press("x")
        await pilot.pause()
        assert not app.is_running


# -- several open questions: drafts, Submit tab ----------------------------------------

async def test_answering_drafts_and_moves_to_the_next_tab(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "header": "First", "options": ["a", "b"]},
                                 {"question": "Two?", "header": "Second", "options": ["c", "d"]}])
    service.ask(store, B, "Three?", options=["e"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("enter")  # open the first group
        assert "Tab/Arrow keys to navigate" in text_of(view(app).query_one(".hint"))
        await pilot.press("right")
        assert view(app).q.id == 2
        await pilot.press("tab")
        assert app.query(ReviewView)
        await pilot.press("tab")
        assert view(app).q.id == 1  # wraps within the group
        await pilot.press("1", "enter")  # answer One -> next tab
        assert view(app).q.id == 2 and "✔ First" in "".join(chips(app))
        await pilot.press("2", "enter")  # Two -> this group's Submit tab
        assert app.query(ReviewView) and [q.id for q in app.query_one(ReviewView).items] == [1, 2]
    assert prompts(fake_herdr) == []  # drafts until Submit


async def test_submit_is_per_group_and_keeps_blanks(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "options": ["a", "b"]}, {"question": "Two?", "options": ["c"]}])
    service.ask_group(store, B, [{"question": "Three?"}, {"question": "Four?", "options": ["x"]}])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("2", "enter")                 # open B's group
        await pilot.press("enter", *"later", "enter")   # Three typed
        await pilot.press("1", "enter")                 # Four=x -> Submit tab
        body = text_of(app.query_one("#review-body"))
        assert "Review your answers" in body and "One?" not in body
        await pilot.press("enter")                      # Submit answers (this group only)
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert app.mode == "list" and app.closed.get(("group", 3)) == "sending…"
        await pilot.press("1", "enter", "1", "enter")   # A's group: One=a, skip Two
        await pilot.press("tab")
        body = text_of(app.query_one("#review-body"))
        assert "You have not answered all questions" in body
        await pilot.press("enter")
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert app.mode == "list" and ("group", 1) not in app.closed  # Two stays open: still openable
    assert [store.get(i).status for i in (1, 2, 3, 4)] == ["answered", "open", "answered", "answered"]
    assert prompts(fake_herdr) == ["[inbox] answered — Q3 Three?: later · Q4 Four?: x",
                                   "[inbox] Q1 answered — One?: a"]


async def test_cancel_and_drafts_survive_a_restart(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "options": ["a", "b"]}, {"question": "Two?", "options": ["c"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        assert app.mode == "group"  # the only group: opened straight away
        await pilot.press("2", "enter", "right", "2")  # One=b, Submit tab, Cancel
        await pilot.pause()
        assert not app.is_running  # opened directly: Cancel/Esc closes the popup
    with Store() as again:
        app = InboxApp(again)
        async with app.run_test() as pilot:
            await opened(app, pilot)
            assert "✔ Q1" in "".join(chips(app)) and view(app).q.id == 2  # opens on the first blank
            await pilot.press("left")
            assert view(app).picked == {1}
            await pilot.press("1", "enter")  # edit
            assert again.get(1).draft_json["choices"] == ["a"]
            await pilot.press("left", "x")  # clear it
            assert again.get(1).draft_json is None
    assert prompts(fake_herdr) == []


async def test_new_groups_never_join_an_open_group(store, fake_herdr):
    """Was 'wait behind a marker': arrivals go to the list; the open group never changes."""
    service.ask_group(store, A, [{"question": "One?"}, {"question": "Two?"}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("right")
        assert view(app).q.id == 2
        service.ask(store, B, "Three?")
        app.poll()
        await pilot.pause()
        assert app.open_qids == [1, 2] and ("group", 3) in app.entries and view(app).q.id == 2
        service.close(store, 1, "withdrawn")
        app.poll()
        await pilot.pause()
        assert app.open_qids == [2] and view(app).q.id == 2
        await pilot.press("escape")  # opened straight into the group: Esc closes
        await pilot.pause()
        assert not app.is_running


async def test_one_becomes_several_while_open(store, fake_herdr):
    """The send mode is decided when the group is opened; a second group arriving changes
    nothing inside it (a lone question still sends on Enter)."""
    service.ask(store, A, "One?", options=["a"])
    service.ask(store, A, "Spare?", options=["s"])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("enter")  # deliberately open the lone One?
        assert chips(app) == []
        service.ask(store, B, "Two?", options=["b"])
        app.poll()
        await pilot.pause()
        assert chips(app) == [] and app.single
        await pilot.press("1", "enter")  # still a deliberate lone question: sends
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert app.mode == "list"
    assert store.get(1).answer == "a" and store.get(3).status == "open"


async def test_dismiss_question_in_group_and_whole_group_in_list(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?"}, {"question": "Two?"}])
    service.ask_group(store, B, [{"question": "Three?"}, {"question": "Four?"}])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await opened(app, pilot)
        await pilot.press("enter", "d")  # inside A's group: one question
        await pilot.pause()
        assert "Dismiss" in text_of(app.screen.query_one("#confirm"))
        await pilot.press("x")  # any other key keeps it
        await pilot.pause()
        assert store.get(1).status == "open" and view(app).q.id == 1
        await pilot.press("d", "y")
        await pilot.pause()
        assert view(app).q.id == 2
        await pilot.press("escape", "down", "d")  # the list: B's whole group
        await pilot.pause()
        assert "Q3–Q4 (2 questions)" in text_of(app.screen.query_one("#confirm"))
        await pilot.press("d")  # not y: kept
        await pilot.pause()
        assert store.get(3).status == "open"
        await pilot.press("d", "y")
        await pilot.pause()
        assert app.closed.get(("group", 3)) == "dismissed"
    assert [store.get(i).status for i in (1, 2, 3, 4)] == ["dismissed", "open", "dismissed", "dismissed"]


async def test_busy_database_never_crashes_the_popup(store, fake_herdr, monkeypatch):
    import sqlite3

    service.ask(store, A, "One?")

    def locked(*args, **kwargs):
        raise sqlite3.OperationalError("database is locked")

    app = InboxApp(store)
    async with app.run_test() as pilot:
        await opened(app, pilot)
        monkeypatch.setattr(service, "close", locked)
        await pilot.press("d")
        await pilot.pause()
        assert app.is_running and store.get(1).status == "open"


async def test_tab_row_scrolls_to_keep_the_current_tab_visible(store, fake_herdr):
    service.ask_group(store, A, [{"question": f"Question number {i}?", "header": f"Header {i:02d}"} for i in range(4)])
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=(40, 20)) as pilot:
        await opened(app, pilot)
        for _ in range(4):
            await pilot.press("right")
        await pilot.pause()
        row, current = app.query_one("#chips"), app.query_one("#chips > .current")
        assert row.scroll_x > 0 and row.region.contains_region(current.region)
