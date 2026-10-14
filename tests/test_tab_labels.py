"""Live tab labels: the popup shows the asking tab's current name, not the one captured at
ask time (an unnamed tab's label is just its position, e.g. "8")."""

import json

import pytest

from herdr_inbox import service, tui
from herdr_inbox.service import Caller
from herdr_inbox.tui import GroupListView, InboxApp, QuestionView

A = Caller("w1:p1", "w1:t1", "8", "sess-A", "/r")  # asked while the tab was unnamed
B = Caller("w1:p2", "w1:t2", "Other", "sess-B", "/r")


@pytest.fixture
def tabs_file(fake_herdr, tmp_path, monkeypatch):
    path = tmp_path / "tabs.json"
    path.write_text(json.dumps({"w1:t1": "8", "w1:t2": "Other"}))
    monkeypatch.setenv("FAKE_HERDR_TABS", str(path))
    monkeypatch.setattr(tui, "LIVE_TAB_LABELS", True)
    tui._tab_labels.clear()
    return path


def rename(path, tab, label):
    data = json.loads(path.read_text())
    data[tab] = label
    path.write_text(json.dumps(data))


async def test_renamed_tab_shows_its_current_name(store, fake_herdr, tabs_file):
    service.ask_group(store, A, [{"question": "One?"}, {"question": "Two?"}])
    service.ask(store, B, "Elsewhere?")
    rename(tabs_file, "w1:t1", "❓2 Test Tab")  # renamed by the operator, with our count prefix
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert "Test Tab" in body and "8 ·" not in body and "❓" not in body
        await pilot.press("enter")
        chips = [str(c.render()) for c in app.query("#chips > Static")]
        assert chips[0] == "│ Test Tab"
        assert "Test Tab" in str(app.query_one(QuestionView).query_one(".meta").render())
    assert store.get(1).tab_label == "8"  # the stored value is only the fallback


async def test_gone_tab_falls_back_to_the_stored_label(store, fake_herdr, tabs_file, monkeypatch):
    service.ask(store, A, "One?")
    service.ask(store, B, "Two?")
    monkeypatch.setenv("FAKE_HERDR_FAIL", "tab get")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert "8 · 1 question" not in app.query_one(GroupListView).body_text()  # name is the question
        assert "\n      8" in app.query_one(GroupListView).body_text()  # second line: stored label


def test_labels_are_cached_briefly(store, fake_herdr, tabs_file):
    service.ask(store, A, "One?")
    q = store.get(1)
    fake_herdr.reset()
    assert tui.tab_name(q, store) == "8"
    rename(tabs_file, "w1:t1", "Renamed")
    assert tui.tab_name(q, store) == "8"  # within the TTL: no second herdr call
    assert len(fake_herdr.calls_of("tab", "get")) == 1
    tui._tab_labels.clear()  # the TTL ran out
    assert tui.tab_name(q, store) == "Renamed"
