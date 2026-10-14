"""Pilot tests for the popup: list, Claude-Code-style question view, groups, drafts, review."""

from textual.widgets import DataTable, Input, Static

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.store import Store
from herdr_inbox.tui import CHAT, OTHER, GroupScreen, InboxApp, QuestionView, ReviewScreen, ReviewView

A = Caller(pane_id="w1:p1", tab_id="w1:t1", tab_label="tab-A", session_id="sess-A", cwd="/r")
B = Caller(pane_id="w1:p2", tab_id="w1:t2", tab_label="tab-B", session_id="sess-A", cwd="/r")

DB = [{"label": "Postgres", "description": "keeps the current ORM"}, {"label": "SQLite", "description": "single file"},
      {"label": "DuckDB"}]


def cells(app):
    table = app.base.query_one(DataTable)
    return [table.get_row_at(i) for i in range(table.row_count)]


def view(app) -> QuestionView:
    return app.screen.query_one(QuestionView)


def text_of(widget) -> str:
    return str(widget.render())


def prompts(fake):
    return [c[3] for c in fake.calls_of("agent", "prompt")]


async def started(app, pilot):
    await pilot.pause()
    return app


# -- list -------------------------------------------------------------------------

async def test_list_renders_groups_newest_first(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB, header="DB choice", asked_by="infra")
    service.ask_group(store, B, [{"question": "Branch name?"}, {"question": "Squash?", "options": ["yes", "no"]},
                                 {"question": "Tag it?"}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        rows = cells(app)
        assert [r[0] for r in rows] == ["Q2–Q4", "Q1"]
        assert rows[0][5] == "3 open" and "(+2 more)" in rows[0][6]
        assert rows[1][1:3] == ["tab-A", "infra"]


# -- a single question: answers send at once ----------------------------------------

async def test_single_question_layout_cursor_and_jump(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB, header="DB choice")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter")
        v = view(app)
        assert "DB choice" in text_of(v.query_one(".chip"))
        assert not app.screen.query("#chips")  # no tabs for a single question
        assert [k for k, _ in v.rows] == ["opt", "opt", "opt", "other", "chat"]
        assert v.row_text(0).startswith("[$accent]❯[/] ") and "keeps the current ORM" in v.row_text(0)
        assert OTHER in v.row_text(3) and CHAT in v.row_text(4)
        await pilot.press("down", "j")
        assert v.cursor == 2
        await pilot.press("k", "4")
        assert v.cursor == 3
        await pilot.press("1")
        assert v.cursor == 0
        hint = text_of(v.query_one(".hint"))
        assert hint == "Enter to select · ↑/↓ to navigate · press n to add notes · Esc to cancel"


async def test_single_pick_sends_immediately(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "2", "n", *"no server", "enter", "enter")  # note, then pick SQLite
        await pilot.pause()
        assert app.screen is app.base
    q = store.get(1)
    assert q.status == "answered" and q.answer == "SQLite (note: no server)"
    assert q.answer_json == {"choices": ["SQLite"], "notes": {"SQLite": "no server"}, "text": None}
    assert prompts(fake_herdr) == ["[inbox] Q1 answered — Which DB?: SQLite (note: no server)"]


async def test_type_something_replaces_the_option_in_single_select(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "4", "enter")
        assert isinstance(app.screen.focused, Input)
        await pilot.press(*"whatever is cheapest", "enter")
        await pilot.pause()
    assert store.get(1).answer == "whatever is cheapest"


async def test_multi_select_toggle_other_and_submit(store, fake_herdr):
    service.ask(store, A, "Which DBs?", options=DB, multi_select=True)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter")
        v = view(app)
        assert [k for k, _ in v.rows][-2:] == ["other", "submit"] and CHAT not in "".join(map(v.row_text, range(5)))
        await pilot.press("enter", "3", "space", "space", "enter")  # 1 on; 3 on/off/on
        assert v.picked == {0, 2} and "[x]" in v.row_text(0) and "[ ]" in v.row_text(1)
        await pilot.press("1", "n", *"keep schema", "enter")
        await pilot.press("4", "enter", *"also redis", "enter")  # Claude Code: added to the picks
        assert v.text == "also redis" and "[x] also redis" in v.row_text(3)
        await pilot.press("down", "enter")  # Submit
        await pilot.pause()
    q = store.get(1)
    assert q.answer == "Postgres (note: keep schema), DuckDB, also redis"
    assert q.answer_json == {"choices": ["Postgres", "DuckDB"], "notes": {"Postgres": "keep schema"}, "text": "also redis"}


async def test_chat_about_this_focuses_the_pane_and_keeps_the_question(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "5", "enter")
        await pilot.pause()
    assert fake_herdr.calls_of("agent", "focus") == [["agent", "focus", "w1:p1"]]
    assert store.get(1).status == "open" and prompts(fake_herdr) == []


async def test_escape_paths(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "n", *"dq", "escape")  # note box: typed d/q are text; Esc cancels it
        assert isinstance(app.screen, GroupScreen) and view(app).notes == {}
        await pilot.press("4", "enter", *"x", "escape")  # Type something.: Esc cancels
        assert isinstance(app.screen, GroupScreen) and view(app).editing is None
        await pilot.press("escape")  # back to the list, nothing sent
        assert app.screen is app.base
        await pilot.press("escape")  # closes the popup
    assert store.get(1).status == "open" and prompts(fake_herdr) == []


async def test_old_string_only_options(store, fake_herdr):
    store.db.execute("INSERT INTO questions (created_at, question, norm, options, pane_id, session_id, status)"
                     " VALUES ('2026-10-14T00:00:00+00:00', 'Legacy?', 'legacy', '[\"yes\", \"no\"]', 'w1:p1', 'sess-A', 'open')")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "2", "enter")
        await pilot.pause()
    assert store.get(1).answer == "no"


async def test_preview_layout(store, fake_herdr):
    service.ask(store, A, "Which config?", options=[
        {"label": "Small", "preview": "workers = 2\nmemory = 1Gi"}, {"label": "Large", "description": "x"}])
    app = InboxApp(store)
    async with app.run_test(size=(100, 30)) as pilot:
        await started(app, pilot)
        await pilot.press("enter")
        preview = app.screen.query_one("#preview", Static)
        assert "workers = 2" in text_of(preview)
        await pilot.press("down")
        assert text_of(preview) == "No preview available" and preview.has_class("empty")
        left, right = app.screen.query_one(".split > .choices"), app.screen.query_one(".preview-box")
        assert left.region.x < right.region.x  # side by side


# -- groups ---------------------------------------------------------------------------

async def test_group_tabs_advance_and_switch(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "header": "First", "options": ["a", "b"]},
                                 {"question": "Two?", "header": "Second", "options": ["c", "d"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter")
        screen = app.screen
        chips = lambda: text_of(screen.query_one("#chips"))  # noqa: E731
        assert view(app).q.id == 1 and "● First" in chips() and "Submit" in chips()
        assert "Tab/Arrow keys to navigate" in text_of(view(app).query_one(".hint"))
        await pilot.press("right")
        assert view(app).q.id == 2
        await pilot.press("tab")
        assert screen.query(ReviewView)
        await pilot.press("tab")
        assert view(app).q.id == 1  # wraps
        await pilot.press("shift+tab", "left")
        assert view(app).q.id == 2
        await pilot.press("left", "1", "enter")  # answering One moves to the next tab
        assert view(app).q.id == 2 and "✓ First" in chips()
        await pilot.press("2", "enter")  # Two answered -> Submit tab
        assert screen.query(ReviewView)
    assert prompts(fake_herdr) == []  # group answers are drafts until submitted


async def test_review_warns_and_partial_submit_leaves_blanks_open(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "options": ["a", "b"]}, {"question": "Two?", "options": ["c", "d"]},
                                 {"question": "Three?"}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "1", "enter")  # One=a -> Two
        await pilot.press("right")  # skip Two -> Three
        await pilot.press("enter", *"later", "enter")  # Three -> Submit tab
        body = text_of(app.screen.query_one("#review-body"))
        assert "Review your answers" in body and "You have not answered all questions" in body
        assert "(not answered)" in body and "Ready to submit your answers?" in body
        assert "Submit answers" in body and "Cancel" in body
        await pilot.press("enter")  # cursor starts on Submit answers
        await pilot.pause()
        assert [r[0] for r in cells(app)] == ["Q2"]
    assert [store.get(i).status for i in (1, 2, 3)] == ["answered", "open", "answered"]
    assert prompts(fake_herdr) == ["[inbox] answered — Q1 One?: a · Q3 Three?: later"]


async def test_review_cancel_keeps_drafts(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "options": ["a"]}, {"question": "Two?", "options": ["b"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "enter", "enter")  # both answered -> Submit tab, no warning
        assert "not answered all" not in text_of(app.screen.query_one("#review-body"))
        await pilot.press("2")  # Cancel
        assert app.screen is app.base and cells(app)[0][5] == "2/2 drafted"
    assert prompts(fake_herdr) == []


async def test_drafts_survive_popup_restart_and_can_be_edited_or_cleared(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "options": ["a", "b"]}, {"question": "Two?", "options": ["c", "d"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "2", "enter", "escape")  # draft One=b, back to the list
        assert cells(app)[0][5] == "1/2 drafted"
    with Store() as again:
        app = InboxApp(again)
        async with app.run_test() as pilot:
            await started(app, pilot)
            assert cells(app)[0][5] == "1/2 drafted"
            await pilot.press("enter")
            assert view(app).q.id == 2  # opens on the first unanswered question
            await pilot.press("left")
            v = view(app)
            assert v.picked == {1} and v.cursor == 1
            await pilot.press("1", "enter")  # edit: a instead of b, moves on to Two
            assert again.get(1).draft_json["choices"] == ["a"]
            await pilot.press("left", "x")  # back to One, clear its draft
            assert again.get(1).draft_json is None
    assert prompts(fake_herdr) == []


async def test_global_review_across_two_panes(store, fake_herdr):
    service.ask_group(store, A, [{"question": "From A?", "options": ["x", "y"]}, {"question": "Also from A?"}])
    service.ask_group(store, B, [{"question": "From B?", "options": ["p", "q"]}, {"question": "More B?"}])
    for qid, detail in ((1, service.make_detail(None, ["x"])), (2, service.make_detail("sure")),
                        (3, service.make_detail(None, ["q"]))):
        store.set_draft(qid, detail)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("r")
        assert isinstance(app.screen, ReviewScreen)
        review = app.screen.query_one(ReviewView)
        assert [q.id for q in review.items] == [1, 2, 3]
        await pilot.press("up", "x")  # cursor: Submit answers -> Q3; drop Q3's draft
        assert store.get(3).draft_json is None and [q.id for q in review.items] == [1, 2]
        await pilot.press("up", "up", "enter")  # edit Q1 -> its group's tab
        assert isinstance(app.screen, GroupScreen) and view(app).q.id == 1
        await pilot.press("2", "enter", "escape")  # change to y, back to the global review
        assert isinstance(app.screen, ReviewScreen)
        await pilot.press("1")  # Submit answers
        await pilot.pause()
    assert [store.get(i).status for i in (1, 2, 3, 4)] == ["answered", "answered", "open", "open"]
    assert prompts(fake_herdr) == ["[inbox] answered — Q1 From A?: y · Q2 Also from A?: sure"]


async def test_dismiss_history_and_goto(store, fake_herdr, monkeypatch):
    service.ask(store, A, "One?")
    service.ask_group(store, B, [{"question": "Two?"}, {"question": "Three?"}])
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("d")  # dismiss the whole group Q2–Q3
        await pilot.pause()
        assert [store.get(i).status for i in (2, 3)] == ["dismissed", "dismissed"]
        await pilot.press("enter", "enter", *"later", "enter")  # Q1: type, sends; delivery fails
        await pilot.pause()
        assert cells(app) == []
        await pilot.press("h")
        await pilot.pause()
        assert {r[0]: r[5] for r in cells(app)} == {"Q3": "dismissed", "Q2": "dismissed", "Q1": "answered !"}
        await pilot.press("down", "down", "g")
        await pilot.pause()
    assert fake_herdr.calls_of("agent", "focus") == [["agent", "focus", "w1:p1"]]


async def test_busy_database_never_crashes_the_popup(store, fake_herdr, monkeypatch):
    import sqlite3

    service.ask(store, A, "One?")
    app = InboxApp(store)

    def locked(*args, **kwargs):
        raise sqlite3.OperationalError("database is locked")

    async with app.run_test() as pilot:
        await started(app, pilot)
        monkeypatch.setattr(service, "close", locked)
        await pilot.press("d")
        await pilot.pause()
        assert app.is_running and store.get(1).status == "open"
