"""Open-question count in the herdr tab label (``❓2 Farm Issues``), via the fake herdr."""

import json

import pytest

from herdr_inbox import service, tabs
from herdr_inbox.service import Caller, resolve_caller


@pytest.fixture
def tabbed(fake_herdr, tmp_path, monkeypatch):
    """Tabs w1:t1 'Farm Issues' (panes w1:p1, w1:p3) and w1:t2 'Proxy' (pane w1:p2); prefix on."""
    path = tmp_path / "tabs.json"
    path.write_text(json.dumps({"w1:t1": "Farm Issues", "w1:t2": "Proxy"}))
    monkeypatch.setenv("FAKE_HERDR_TABS", str(path))
    monkeypatch.delenv("HERDR_INBOX_TAB_PREFIX")
    monkeypatch.setenv("FAKE_HERDR_CONFIG_DIR", str(tmp_path / "cfg"))
    tabs._file_settings.cache_clear()
    yield path
    tabs._file_settings.cache_clear()


def labels(path):
    return json.loads(path.read_text())


def renames(fake):
    return [c[2:] for c in fake.calls_of("tab", "rename")]


def caller(pane="w1:p1", tab="w1:t1", label="Farm Issues", session="sess-A"):
    return Caller(pane_id=pane, tab_id=tab, tab_label=label, session_id=session, cwd="/r")


def test_prefix_added_updated_and_restored(store, fake_herdr, tabbed):
    a = service.ask(store, caller(), "One?")["id"]
    assert labels(tabbed)["w1:t1"] == "❓1 Farm Issues"
    b = service.ask(store, caller(pane="w1:p3"), "Two?")["id"]  # another pane in the same tab
    assert labels(tabbed)["w1:t1"] == "❓2 Farm Issues"
    service.answer(store, a, "yes")
    assert labels(tabbed)["w1:t1"] == "❓1 Farm Issues"
    service.close(store, b, "dismissed")
    assert labels(tabbed) == {"w1:t1": "Farm Issues", "w1:t2": "Proxy"}  # original restored, other tab untouched
    assert renames(fake_herdr) == [["w1:t1", "❓1 Farm Issues"], ["w1:t1", "❓2 Farm Issues"],
                                   ["w1:t1", "❓1 Farm Issues"], ["w1:t1", "Farm Issues"]]


def test_no_rename_when_label_already_right(store, fake_herdr, tabbed):
    service.ask(store, caller(), "One?")
    fake_herdr.reset()
    service.update_markers(store, "w1:p1")
    assert renames(fake_herdr) == []
    assert ["tab", "get", "w1:t1"] in fake_herdr.calls()  # re-read before deciding


def test_operator_rename_meanwhile_is_adopted(store, fake_herdr, tabbed):
    a = service.ask(store, caller(), "One?")["id"]
    service.ask(store, caller(), "Two?")
    tabs_now = labels(tabbed)
    tabs_now["w1:t1"] = "Farm v2"  # operator renamed the prefixed tab
    tabbed.write_text(json.dumps(tabs_now))
    service.answer(store, a, "ok")
    assert labels(tabbed)["w1:t1"] == "❓1 Farm v2"
    tabs_now = labels(tabbed)
    tabs_now["w1:t1"] = "❓1 Farm v3"  # renamed keeping our prefix: still only one prefix
    tabbed.write_text(json.dumps(tabs_now))
    service.close(store, 2, "dismissed")
    assert labels(tabbed)["w1:t1"] == "Farm v3"


def test_question_stores_the_base_label(store, fake_herdr, tabbed, monkeypatch):
    service.ask(store, caller(), "One?")
    c = resolve_caller()  # HERDR_PANE_ID=w1:p1, its tab now reads '❓1 Farm Issues'
    assert c.tab_label == "Farm Issues"


def test_unnamed_tab_is_left_alone_by_default(store, fake_herdr, tabbed, tmp_path):
    tabbed.write_text(json.dumps({"w1:t1": "6", "w1:t2": "Proxy"}))
    service.ask(store, caller(label="6"), "One?")
    assert labels(tabbed)["w1:t1"] == "6" and renames(fake_herdr) == []
    # opt in through the plugin config file
    (tmp_path / "cfg").mkdir()
    (tmp_path / "cfg" / "config.toml").write_text("prefix_unnamed_tabs = true\n")
    tabs._file_settings.cache_clear()
    service.update_markers(store, "w1:p1")
    assert labels(tabbed)["w1:t1"] == "❓1 6"


def test_prefix_configurable_and_disableable(store, fake_herdr, tabbed, tmp_path, monkeypatch):
    (tmp_path / "cfg").mkdir()
    (tmp_path / "cfg" / "config.toml").write_text('tab_prefix = "Q"\n')
    tabs._file_settings.cache_clear()
    service.ask(store, caller(), "One?")
    assert labels(tabbed)["w1:t1"] == "Q1 Farm Issues"
    monkeypatch.setenv("HERDR_INBOX_TAB_PREFIX", "off")  # env wins over the file
    fake_herdr.reset()
    service.ask(store, caller(), "Two?")
    assert renames(fake_herdr) == []  # disabled: no tab work on the hot path
    service.sync_tokens(store)  # ...and the reconcile removes what is left
    assert labels(tabbed)["w1:t1"] == "Farm Issues"


def test_reconcile_adds_missing_and_removes_stale(store, fake_herdr, tabbed):
    service.ask(store, caller(), "One?")
    # after a crash/restart: t1 lost its prefix, t2 kept a stale one
    tabbed.write_text(json.dumps({"w1:t1": "Farm Issues", "w1:t2": "❓3 Proxy"}))
    service.sync_tokens(store)
    assert labels(tabbed) == {"w1:t1": "❓1 Farm Issues", "w1:t2": "Proxy"}
    fake_herdr.reset()
    service.sync_tokens(store)
    assert renames(fake_herdr) == []  # idempotent


def test_base_label_strips_only_our_prefix():
    assert tabs.base_label("❓12 Farm Issues") == "Farm Issues"
    assert tabs.base_label("❓ not ours") == "❓ not ours"
    assert tabs.base_label("Farm ❓1 x") == "Farm ❓1 x"
    assert tabs.base_label(None) is None
