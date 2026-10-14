from textual.widgets import DataTable, Input, Static

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp


def seed(store):
    c = Caller(pane_id="w1:p1", tab_id="w1:t1", tab_label="my-tab", session_id="sess-A", cwd="/repo")
    service.ask(store, c, "Which region for the bucket?", options=["eu-west-1", "us-east-1"], asked_by="infra")
    c2 = Caller(pane_id="w1:p2", tab_id="w1:t2", tab_label="other-tab", session_id="sess-B", cwd="/repo")
    service.ask(store, c2, "Squash or merge PR 12?", context="PR has 40 commits")


def cells(app):
    table = app.query_one(DataTable)
    return [table.get_row_at(i) for i in range(table.row_count)]


async def test_list_renders_open_questions_newest_first(store, fake_herdr):
    seed(store)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        rows = cells(app)
        assert [r[0] for r in rows] == ["Q2", "Q1"]
        assert rows[1][1:3] == ["my-tab", "infra"]
        assert "Squash or merge PR 12?" in rows[0][6]
        detail = str(app.query_one("#detail", Static).render())
        assert "PR has 40 commits" in detail


async def test_free_text_answer_via_keys(store, fake_herdr):
    seed(store)
    fake_herdr.reset()
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("a")
        assert app.query_one(Input).has_focus
        await pilot.press(*"rebase", "enter")
        await pilot.pause()
        assert [r[0] for r in cells(app)] == ["Q1"]  # answered one left the open list
    q = store.get(2)
    assert q.status == "answered" and q.answer == "rebase" and q.delivered_at
    assert ["agent", "prompt", "w1:p2", "[inbox] Q2 answered — Squash or merge PR 12?: rebase"] in fake_herdr.calls()


async def test_number_key_picks_option(store, fake_herdr):
    seed(store)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down")  # Q1 has the options
        await pilot.press("2")
        assert app.query_one(Input).value == "us-east-1"
        await pilot.press("enter")
        await pilot.pause()
    assert store.get(1).answer == "us-east-1"
    assert fake_herdr.calls_of("agent", "prompt")[-1][2] == "w1:p1"


async def test_escape_cancels_answer_box(store, fake_herdr):
    seed(store)
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("a", "d", "escape")  # 'd' is typed, not a dismiss
        await pilot.pause()
        assert not app.query_one(Input).has_focus
    assert store.get(2).status == "open"


async def test_dismiss_history_and_goto(store, fake_herdr, monkeypatch):
    seed(store)
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("d")  # dismiss Q2
        await pilot.pause()
        assert store.get(2).status == "dismissed"
        await pilot.press("a", *"later", "enter")  # Q1, delivery fails
        await pilot.pause()
        assert cells(app) == []
        await pilot.press("h")
        await pilot.pause()
        states = {r[0]: r[5] for r in cells(app)}
        assert states == {"Q2": "dismissed", "Q1": "answered !"}
        await pilot.press("down", "g")  # Q1 lives in w1:p1
        await pilot.pause()
    assert fake_herdr.calls_of("agent", "focus") == [["agent", "focus", "w1:p1"]]
    assert "fake failure" in store.get(1).delivery_error
