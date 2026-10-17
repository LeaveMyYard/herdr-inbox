"""0.5: agents name each ask_operator call; the popup lists groups by that name."""


from mcp import Client

from herdr_inbox import service
from herdr_inbox.hook import SESSION_RULES
from herdr_inbox.server import mcp
from herdr_inbox.service import Caller
from herdr_inbox.store import MAX_TITLE, Store, clean_title
from herdr_inbox.tui import GroupListView, InboxApp

from . import compat_v01_store as v01

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")


def test_title_is_stored_per_group_in_the_side_table(store, fake_herdr, tmp_path):
    out = service.ask_group(store, A, [{"question": "Workers?", "header": "Workers"}, {"question": "Memory?"}],
                            title="Worker rollout settings")
    assert [store.get(q["id"]).group_title for q in out["questions"]] == ["Worker rollout settings"] * 2
    assert store.db.execute("SELECT title FROM question_groups WHERE group_id = ?", (out["group"],)).fetchone()[0] \
        == "Worker rollout settings"
    # the questions table keeps the 0.1.x column set, and 0.1.x still reads every row
    columns = [r[1] for r in store.db.execute("PRAGMA table_info(questions)")]
    assert "title" not in columns and "group_title" not in columns
    old = v01.Store(store.path)
    assert sorted(q.id for q in old.list()) == [1, 2]


def test_title_is_sanitised():
    assert clean_title("  Worker\nrollout\x1b[31m\tsettings  ") == "Worker rollout [31m settings"
    long = clean_title("x" * 100)
    assert len(long) == MAX_TITLE and long.endswith("…")
    assert clean_title("   ") is None and clean_title(None) is None


def test_fallback_name_for_untitled_groups(store, fake_herdr):
    service.ask(store, A, "Which region?\nmore", header="Region")
    service.ask(store, B, "Ship it today?\ndetails")
    service.ask_group(store, A, [{"question": "One?"}, {"question": "Two?", "header": "Second"}])
    assert service.group_name(store.group(1)) == "Region"          # single: its header
    assert service.group_name(store.group(2)) == "Ship it today?"  # else the first line
    assert service.group_name(store.group(3)) == "One?"            # old-style group without a title


async def test_mcp_title_parameter(migrated_db, fake_herdr):
    async with Client(mcp) as client:
        tools = {t.name: t for t in (await client.list_tools()).tools}
        desc = tools["ask_operator"].description
        assert "`title`" in desc and "ALWAYS set it when you pass `questions`" in desc
        r = (await client.call_tool("ask_operator", {"title": "Worker rollout settings", "questions": [
            {"question": "Workers?"}, {"question": "Memory?"}]})).structured_content
        single = (await client.call_tool("ask_operator", {"question": "Lone?", "title": "Lone call"})).structured_content
    with Store() as s:
        assert s.group(r["group"])[0].group_title == "Worker rollout settings"
        assert s.get(single["id"]).group_title == "Lone call"


def test_session_start_text_mentions_title_once_and_stays_short():
    assert SESSION_RULES.count("`title`") == 1 and len(SESSION_RULES.split()) < 250


async def test_list_row_and_group_view_show_the_title(store, fake_herdr):
    service.ask_group(store, A, [{"question": "Workers?", "header": "Workers", "options": ["2", "8"]},
                                 {"question": "Memory?", "header": "Memory"}],
                      title="Worker rollout settings", asked_by="planner")
    service.ask(store, B, "Ship it?", header="Ship")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        lv = app.query_one(GroupListView)
        row = lv.row_text(1, ("group", 1)).replace(" ", " ")
        line1, line2 = row.split("\n")
        assert "Worker rollout settings · 2 questions" in line1
        assert "tab-A · planner · Workers · Memory" in line2
        await pilot.press("enter")
        title = app.query_one("#group-title")
        assert title.display and str(title.render()) == "Worker rollout settings"
        await pilot.press("escape", "down", "enter")  # the untitled lone question: no title band
        assert not app.query_one("#group-title").display


async def test_list_follows_the_cursor_in_a_narrow_popup(store, fake_herdr):
    """The reviewer's 40x14 repro: 8 titled groups, long titles; after 7x down the cursor's
    row must be on screen."""
    title = "W" * 58 + "‮\x1b[31m[b]x" + "Z" * 30
    for i in range(8):
        c = Caller(f"w1:p{i}", "w1:t1", "a-rather-long-tab-label-here", f"s{i}", "/r")
        service.ask_group(store, c, [{"question": f"question {i} " + "lorem ipsum " * 8, "header": "Hdr"},
                                     {"question": "second?"}], title=title + str(i), asked_by="reviewer of something long")
    app = InboxApp(store, focus_pane=None)
    async with app.run_test(size=(40, 14)) as pilot:
        await pilot.pause()
        for _ in range(7):
            await pilot.press("down")
        await pilot.pause()
        lv = app.query_one(GroupListView)
        box = lv.query_one("#list-rows")
        rows = list(lv.query(".entry"))
        current = rows[app.entries.index(lv.cursor)]
        assert lv.cursor == app.entries[7] and box.scroll_y > 0
        assert box.region.contains_region(current.region)


def test_notification_for_an_untitled_single_question_shows_header_and_text(store, fake_herdr):
    service.ask(store, A, "Which database should the invoice store use?", header="DB choice")
    service.ask_group(store, B, [{"question": "Workers?"}, {"question": "Memory?"}], title="Worker rollout settings")
    bodies = [c[4] for c in fake_herdr.calls_of("notification", "show")]
    assert bodies == ["DB choice: Which database should the invoice store use?", "Worker rollout settings"]


async def test_untitled_group_is_named_after_its_first_open_question(store, fake_herdr):
    service.ask_group(store, A, [{"question": "Withdrawn one?", "header": "Old"}, {"question": "Still open?",
                                                                                      "header": "Open"}])
    service.ask(store, B, "Other?")
    service.close(store, 1, "withdrawn")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        row = app.query_one(GroupListView).row_text(1, ("group", 1))
        assert "Open · 1 question" in row.replace(" ", " ") and "Old" not in row
