"""Open-question count in the herdr tab label (``❓2 Billing API``), via the fake herdr."""

import json
import os
import threading
import time
from pathlib import Path

import pytest

from herdr_inbox import cli, herdr, service, tabs
from herdr_inbox.service import Caller, resolve_caller
from herdr_inbox.store import Store


def set_config(text: str) -> None:
    (Path(os.environ["HERDR_PLUGIN_CONFIG_DIR"]) / "config.toml").write_text(text)


@pytest.fixture
def tabbed(fake_herdr, tmp_path, monkeypatch):
    """Tabs w1:t1 'Billing API' (panes w1:p1, w1:p3) and w1:t2 'Docs site' (pane w1:p2); prefix on."""
    path = tmp_path / "tabs.json"
    path.write_text(json.dumps({"w1:t1": "Billing API", "w1:t2": "Docs site"}))
    monkeypatch.setenv("FAKE_HERDR_TABS", str(path))
    set_config("")  # defaults: tab_prefix = "❓"
    return path


def labels(path):
    return json.loads(path.read_text())


def relabel(path, **changes):
    data = labels(path)
    data.update({k.replace("_", ":"): v for k, v in changes.items()})
    path.write_text(json.dumps(data))


def renames(fake):
    return [c[2:] for c in fake.calls_of("tab", "rename")]


def caller(pane="w1:p1", tab="w1:t1", label="Billing API", session="aaaaaaaa-0000-4000-8000-00000000000a"):
    return Caller(pane_id=pane, tab_id=tab, tab_label=label, session_id=session, cwd="/r")


def test_prefix_added_updated_and_restored(store, fake_herdr, tabbed):
    a = service.ask(store, caller(), "One?")["id"]
    assert labels(tabbed)["w1:t1"] == "❓1 Billing API"
    b = service.ask(store, caller(pane="w1:p3"), "Two?")["id"]  # another pane in the same tab
    assert labels(tabbed)["w1:t1"] == "❓2 Billing API"
    service.answer(store, a, "yes")
    assert labels(tabbed)["w1:t1"] == "❓1 Billing API"
    service.close(store, b, "dismissed")
    assert labels(tabbed) == {"w1:t1": "Billing API", "w1:t2": "Docs site"}  # original restored, other tab untouched
    assert renames(fake_herdr) == [["w1:t1", "❓1 Billing API"], ["w1:t1", "❓2 Billing API"],
                                   ["w1:t1", "❓1 Billing API"], ["w1:t1", "Billing API"]]


def test_no_rename_when_label_already_right(store, fake_herdr, tabbed):
    service.ask(store, caller(), "One?")
    fake_herdr.reset()
    service.update_markers(store, "w1:p1")
    assert renames(fake_herdr) == []
    assert ["tab", "get", "w1:t1"] in fake_herdr.calls()  # re-read before deciding


def test_operator_rename_meanwhile_is_adopted(store, fake_herdr, tabbed):
    a = service.ask(store, caller(), "One?")["id"]
    service.ask(store, caller(), "Two?")
    relabel(tabbed, w1_t1="Billing v2")  # operator renamed the prefixed tab
    service.answer(store, a, "ok")
    assert labels(tabbed)["w1:t1"] == "❓1 Billing v2"
    relabel(tabbed, w1_t1="❓1 Billing v3")  # renamed keeping our prefix: still only one prefix
    service.close(store, 2, "dismissed")
    assert labels(tabbed)["w1:t1"] == "Billing v3"


def test_question_stores_the_base_label(store, fake_herdr, tabbed):
    service.ask(store, caller(), "One?")
    assert resolve_caller().tab_label == "Billing API"  # its tab now reads '❓1 Billing API'


def test_unnamed_tab_is_left_alone_by_default(store, fake_herdr, tabbed):
    relabel(tabbed, w1_t1="6")
    service.ask(store, caller(label="6"), "One?")
    assert labels(tabbed)["w1:t1"] == "6" and renames(fake_herdr) == []
    set_config("prefix_unnamed_tabs = true\n")
    service.update_markers(store, "w1:p1")
    assert labels(tabbed)["w1:t1"] == "❓1 6"


def test_prefix_configurable_and_disableable(store, fake_herdr, tabbed):
    set_config('tab_prefix = "Q"\n')
    service.ask(store, caller(), "One?")
    assert labels(tabbed)["w1:t1"] == "Q1 Billing API"
    set_config('tab_prefix = ""\n')
    fake_herdr.reset()
    service.ask(store, caller(), "Two?")
    assert renames(fake_herdr) == [] and not fake_herdr.calls_of("pane", "list")  # no tab work when off
    service.sync_tokens(store)  # ...and the reconcile removes what is left
    assert labels(tabbed)["w1:t1"] == "Billing API"


def test_changing_marks_never_stacks_prefixes(store, fake_herdr, tabbed):
    """M1: every applied mark is recorded and stripped, repeatedly."""
    set_config('tab_prefix = "Q"\n')
    service.ask(store, caller(), "One?")
    assert labels(tabbed)["w1:t1"] == "Q1 Billing API"
    for mark in ("❓", "X", "Q", "❓"):  # settings flip, e.g. config edits between processes
        set_config(f'tab_prefix = "{mark}"\n')
        service.update_markers(store, "w1:p1")
        assert labels(tabbed)["w1:t1"] == f"{mark}1 Billing API"
    relabel(tabbed, w1_t1="❓1 Q1 X1 Billing API")  # a stack left by an older build heals
    service.update_markers(store, "w1:p1")
    assert labels(tabbed)["w1:t1"] == "❓1 Billing API"
    assert tabs.marks_for(store, "w1:t1") == {"Q", "❓", "X"}
    assert resolve_caller().tab_label == "Billing API"


def test_hung_herdr_never_blocks_the_database(store, fake_herdr, tabbed, monkeypatch):
    """N1 (reviewer's test_lock.py): herdr hanging on every call must not hold the SQLite lock."""
    real = fake_herdr.bin.read_text()
    fake_herdr.bin.write_text(real.replace(
        "args = sys.argv[1:]",
        "args = sys.argv[1:]\nimport time\nif args[:2] in (['pane','report-metadata'], ['tab','get'], ['tab','rename']): time.sleep(30)"))
    monkeypatch.setattr(herdr, "TIMEOUT", 0.6)
    qid = service.ask(store, caller(), "Seed?")["id"]
    worker = threading.Thread(target=lambda: service.sync_tokens(Store()))
    worker.start()
    time.sleep(0.3)
    other = Store()
    t0 = time.monotonic()
    assert other.set_draft(qid, service.make_detail("x"))
    other.db.execute("BEGIN IMMEDIATE")
    other.db.execute("ROLLBACK")
    assert time.monotonic() - t0 < 0.5  # writers never wait on herdr
    worker.join(30)


def test_concurrent_updates_never_leave_a_stale_count(store, fake_herdr, tabbed, monkeypatch):
    """M2: count -> tab get -> rename happens under one cross-process lock."""
    first = service.ask(store, caller(), "One?")["id"]
    assert store.set_status(first, "dismissed")  # count is now 0; markers not refreshed yet
    real_tab_label = herdr.tab_label
    in_window = threading.Event()

    def slow_tab_label(tab_id):
        label = real_tab_label(tab_id)
        if threading.current_thread().name == "closer":
            in_window.set()
            time.sleep(0.5)  # the other process files a question meanwhile
        return label

    monkeypatch.setattr(herdr, "tab_label", slow_tab_label)

    def closer():
        with Store() as s:
            service.update_markers(s, "w1:p1")  # computes count 0, renames last without the lock

    def asker():
        with Store() as s:
            service.ask(s, caller(pane="w1:p3"), "Two?")

    t1 = threading.Thread(target=closer, name="closer")
    t1.start()
    in_window.wait(2)
    t2 = threading.Thread(target=asker, name="asker")
    t2.start()
    t1.join(10)
    t2.join(10)
    assert labels(tabbed)["w1:t1"] == "❓1 Billing API"


def test_reconcile_adds_missing_and_removes_stale(store, fake_herdr, tabbed):
    service.ask(store, caller(), "One?")
    relabel(tabbed, w1_t1="Billing API", w1_t2="❓3 Docs site")  # after a crash/restart
    service.sync_tokens(store)
    assert labels(tabbed) == {"w1:t1": "❓1 Billing API", "w1:t2": "Docs site"}
    fake_herdr.reset()
    service.sync_tokens(store)
    assert renames(fake_herdr) == []  # idempotent


def test_remove_prefixes_before_uninstall(store, fake_herdr, tabbed):
    service.ask(store, caller(), "One?")
    assert cli.main(["sync-tokens", "--remove-prefixes"]) == 0
    assert labels(tabbed)["w1:t1"] == "Billing API"


def test_custom_mark_never_touches_operator_labels(store, fake_herdr, tabbed):
    """N2: with tab_prefix = "Q", a tab the operator named "Q4 roadmap" is never stripped."""
    set_config('tab_prefix = "Q"\n')
    relabel(tabbed, w1_t2="Q4 roadmap")  # never decorated by us
    qid = service.ask(store, caller(), "One?")["id"]
    assert labels(tabbed)["w1:t1"] == "Q1 Billing API"
    service.sync_tokens(store)
    assert labels(tabbed)["w1:t2"] == "Q4 roadmap"
    service.close(store, qid, "dismissed")
    assert labels(tabbed)["w1:t1"] == "Billing API" and tabs.marks_for(store, "w1:t1") == {"❓"}
    relabel(tabbed, w1_t1="Q7 plan")  # after we restored it, the operator picks a Q-name
    service.sync_tokens(store)
    service.ask(store, caller(), "Two?")
    assert labels(tabbed) == {"w1:t1": "Q1 Q7 plan", "w1:t2": "Q4 roadmap"}


def test_base_label_strips_only_our_marks():
    marks = {"❓", "Q"}
    assert tabs.base_label("❓12 Billing API", marks) == "Billing API"
    assert tabs.base_label("❓1 Q2 ❓3 Billing", marks) == "Billing"
    assert tabs.base_label("❓ not ours", marks) == "❓ not ours"
    assert tabs.base_label("Billing ❓1 x", marks) == "Billing ❓1 x"
    assert tabs.base_label(None, marks) is None
