"""Pilot tests for the popup: list, AskUserQuestion-style question view, drafts, review."""

from textual.widgets import DataTable, Input, Static

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.store import Store
from herdr_inbox.tui import GroupScreen, InboxApp, QuestionView, ReviewScreen, ReviewView

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


async def started(app, pilot):
    await pilot.pause()
    return app


# -- list -----------------------------------------------------------------------

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


# -- question view ----------------------------------------------------------------

async def test_question_view_layout_cursor_and_number_jump(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB, header="DB choice")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter")
        assert isinstance(app.screen, GroupScreen)
        v = view(app)
        assert "DB choice" in text_of(app.screen.query_one("#chips")) and "Review" in text_of(app.screen.query_one("#chips"))
        first = v.row_text(0)
        assert first.startswith("[b $accent]❯[/] 1.") and "keeps the current ORM" in first
        assert "Type something…" in v.row_text(3)
        await pilot.press("down", "j")
        assert v.cursor == 2
        await pilot.press("k")
        assert v.cursor == 1
        await pilot.press("4")
        assert v.cursor == 3
        await pilot.press("1")
        assert v.cursor == 0
        hint = text_of(v.query_one(".hint"))
        assert "1-9 jump" in hint and "n note" in hint and "Esc back" in hint


async def test_single_pick_drafts_then_review_submits(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "2", "enter")  # pick SQLite -> draft, jump to Review
        assert store.get(1).status == "open" and store.get(1).draft_json["choices"] == ["SQLite"]
        assert fake_herdr.calls_of("agent", "prompt") == []  # a draft is not sent
        review = app.screen.query_one(ReviewView)
        assert review.cursor == len(review.items)  # on Submit
        await pilot.press("enter")
        await pilot.pause()
    q = store.get(1)
    assert q.status == "answered" and q.answer == "SQLite" and q.answer_json["choices"] == ["SQLite"]
    assert fake_herdr.calls_of("agent", "prompt")[-1][3] == "[inbox] Q1 answered — Which DB?: SQLite"


async def test_multi_select_toggle_and_note(store, fake_herdr):
    service.ask(store, A, "Which DBs?", options=DB, multi_select=True)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter")
        v = view(app)
        await pilot.press("enter")  # nothing toggled yet: stays
        assert store.get(1).draft_json is None and isinstance(app.screen, GroupScreen)
        await pilot.press("space", "3", "space", "space", "space")  # 1 on, 3 on/off/on
        assert v.picked == {0, 2}
        assert "[x]" in v.row_text(0) and "[ ]" in v.row_text(1)
        await pilot.press("1", "n", *"keep schema", "enter")  # note on Postgres
        assert v.notes == {0: "keep schema"} and "note: keep schema" in v.row_text(0)
        await pilot.press("enter")  # save picks
        await pilot.press("enter")  # Submit on Review
        await pilot.pause()
    q = store.get(1)
    assert q.answer == "Postgres (note: keep schema), DuckDB"
    assert q.answer_json == {"choices": ["Postgres", "DuckDB"], "notes": {"Postgres": "keep schema"}, "text": None}


async def test_free_text_answer(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "4", "enter")
        assert isinstance(app.screen.focused, Input)
        await pilot.press(*"whatever is cheapest", "enter", "enter")
        await pilot.pause()
    assert store.get(1).answer == "whatever is cheapest"


async def test_escape_paths(store, fake_herdr):
    service.ask(store, A, "Which DB?", options=DB)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "n", *"dq", "escape")  # note box: typed d/q are text; Esc cancels it
        assert isinstance(app.screen, GroupScreen) and view(app).notes == {}
        assert store.get(1).status == "open"
        await pilot.press("4", "enter", *"x", "escape")  # free-text box: Esc cancels
        assert isinstance(app.screen, GroupScreen) and view(app).editing is None
        await pilot.press("escape")  # back to the list
        assert app.screen is app.base
        await pilot.press("escape")  # closes the popup
    assert store.get(1).status == "open" and store.get(1).draft_json is None


async def test_old_string_only_options(store, fake_herdr):
    # rows written before options were objects
    store.db.execute("INSERT INTO questions (created_at, question, norm, options, pane_id, session_id, status)"
                     " VALUES ('2026-10-14T00:00:00+00:00', 'Legacy?', 'legacy', '[\"yes\", \"no\"]', 'w1:p1', 'sess-A', 'open')")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "2", "enter", "enter")
        await pilot.pause()
    assert store.get(1).answer == "no"


# -- groups and drafts -------------------------------------------------------------

async def test_group_chip_navigation(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "header": "First", "options": ["a", "b"]},
                                 {"question": "Two?", "header": "Second", "options": ["c", "d"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter")
        screen = app.screen
        assert view(app).q.id == 1 and "● First" in text_of(screen.query_one("#chips"))
        await pilot.press("right")
        assert view(app).q.id == 2
        await pilot.press("tab")
        assert screen.query(ReviewView)
        await pilot.press("tab")
        assert view(app).q.id == 1  # wraps
        await pilot.press("shift+tab")
        assert screen.query(ReviewView)
        await pilot.press("left")
        assert view(app).q.id == 2
        await pilot.press("left", "1", "enter")  # draft One -> moves to the undrafted Two
        assert view(app).q.id == 2 and "✓ First" in text_of(screen.query_one("#chips"))


async def test_drafts_survive_popup_restart_and_can_be_edited_or_cleared(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "options": ["a", "b"]}, {"question": "Two?", "options": ["c", "d"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "2", "enter", "escape")  # draft One=b, close the view
        assert cells(app)[0][5] == "1/2 drafted"
    # a new popup (fresh connection) sees the draft
    with Store() as again:
        app = InboxApp(again)
        async with app.run_test() as pilot:
            await started(app, pilot)
            assert cells(app)[0][5] == "1/2 drafted"
            await pilot.press("enter")
            assert view(app).q.id == 2  # opens on the first undrafted question
            await pilot.press("left")
            v = view(app)
            assert v.picked == {1} and "✓" in v.row_text(1)
            await pilot.press("1", "enter")  # edit: a instead of b
            assert again.get(1).draft_json["choices"] == ["a"]
            assert view(app).q.id == 2  # editing moved on to the undrafted Two
            await pilot.press("left", "x")  # back to One, clear its draft
            assert again.get(1).draft_json is None
    assert fake_herdr.calls_of("agent", "prompt") == []


async def test_partial_submit_leaves_undrafted_open(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "options": ["a", "b"]}, {"question": "Two?", "options": ["c", "d"]},
                                 {"question": "Three?"}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("enter", "1", "enter")  # One=a, moves to Two
        await pilot.press("right")  # skip Two, go to Three
        await pilot.press("enter", *"later", "enter")  # Three: free text, then Review (Two is undrafted)
        review = app.screen.query_one(ReviewView)
        body = text_of(review.query_one("#review-body"))
        assert "no answer yet: stays open" in body and "Submit 2 answers" in body
        await pilot.press("enter")
        await pilot.pause()
        assert [r[0] for r in cells(app)] == ["Q2"]
    assert [store.get(i).status for i in (1, 2, 3)] == ["answered", "open", "answered"]
    prompts = fake_herdr.calls_of("agent", "prompt")
    assert prompts == [["agent", "prompt", "w1:p1", "[inbox] answered — Q1 One?: a · Q3 Three?: later"]]


async def test_global_review_across_two_panes(store, fake_herdr):
    service.ask(store, A, "From A?", options=["x", "y"])
    service.ask(store, B, "From B?", options=["p", "q"])
    service.ask(store, A, "Also from A?")
    for qid, detail in ((1, {"choices": ["x"]}), (2, {"choices": ["q"]}), (3, {"text": "sure"})):
        store.set_draft(qid, service.make_detail(detail.get("text"), detail.get("choices")))
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("r")
        assert isinstance(app.screen, ReviewScreen)
        review = app.screen.query_one(ReviewView)
        assert [q.id for q in review.items] == [1, 2, 3]
        await pilot.press("up", "up", "x")  # drop Q2's draft (cursor: Submit -> Q3 -> Q2)
        assert store.get(2).draft_json is None and [q.id for q in review.items] == [1, 3]
        await pilot.press("up", "enter")  # edit Q1 -> its group view
        assert isinstance(app.screen, GroupScreen) and view(app).q.id == 1
        await pilot.press("2", "enter")  # change to y -> Review chip of that group
        await pilot.press("escape")  # back to the global review
        assert isinstance(app.screen, ReviewScreen)
        await pilot.press("down", "down", "enter")  # Q1 -> Q3 -> Submit
        await pilot.pause()
    assert [store.get(i).status for i in (1, 2, 3)] == ["answered", "open", "answered"]
    assert store.get(1).answer == "y"
    prompts = {c[2]: c[3] for c in fake_herdr.calls_of("agent", "prompt")}
    assert prompts == {"w1:p1": "[inbox] answered — Q1 From A?: y · Q3 Also from A?: sure"}


async def test_dismiss_history_and_goto(store, fake_herdr, monkeypatch):
    service.ask(store, A, "One?")
    service.ask(store, B, "Two?")
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await started(app, pilot)
        await pilot.press("d")  # dismiss Q2 (newest row)
        await pilot.pause()
        assert store.get(2).status == "dismissed"
        await pilot.press("enter", "enter", *"later", "enter", "enter")  # Q1 free text, submit; delivery fails
        await pilot.pause()
        assert cells(app) == []
        await pilot.press("h")
        await pilot.pause()
        assert {r[0]: r[5] for r in cells(app)} == {"Q2": "dismissed", "Q1": "answered !"}
        await pilot.press("down", "g")
        await pilot.pause()
    assert fake_herdr.calls_of("agent", "focus") == [["agent", "focus", "w1:p1"]]
