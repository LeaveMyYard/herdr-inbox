"""Live 2026-10-16 (Q155): the asking agent's pane moved (w6:p48 -> w6:p57), so delivery
keyed by pane id found no agent and marked the answer gone. Before that, the agent is looked
up by its (continuation-followed) session among herdr's agents."""

import json

import pytest

from herdr_inbox import service, store as st, worker
from herdr_inbox.service import Caller

from .test_session_continuation import NEW, OLD, continued


def agents(*pairs):
    return json.dumps([{"pane_id": p, "tab_id": "w1:t9", "agent": "claude", "agent_status": "idle",
                        "agent_session": {"value": s}} for p, s in pairs])


@pytest.fixture(autouse=True)
def _fresh_backoff():
    getattr(service, "_revive_checked", {}).clear()


@pytest.fixture
def moved(fake_herdr, monkeypatch):
    """The question's pane w1:p1 is gone; the conversation now runs in w1:p7."""
    monkeypatch.setenv("FAKE_HERDR_GONE_PANES", "w1:p1")
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w1:p7", "aaaaaaaa-0000-4000-8000-00000000000a"), ("w1:p8", "bbbbbbbb-0000-4000-8000-00000000000b")))
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)


def answered(store, session="aaaaaaaa-0000-4000-8000-00000000000a"):
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", session, "/r"), "Where now?")["id"]
    service.answer(store, qid, "here")
    return qid


def test_an_answer_follows_its_agent_to_the_pane_it_moved_to(store, moved, fake_herdr):
    qid = answered(store)
    q = store.get(qid)
    assert q.pane_id == "w1:p7" and q.tab_id == "w1:t9" and "moved" in (q.delivery_error or "")
    assert q.tab_label == "my-tab"  # the new tab's label, not the one the agent left (#42 Low)
    worker.tick(store)
    assert ["agent", "prompt", "w1:p7"] == fake_herdr.calls_of("agent", "prompt")[-1][:3]
    assert store.get(qid).delivered_at


def test_no_agent_with_that_session_anywhere_is_still_no_agent(store, moved, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w1:p8", "bbbbbbbb-0000-4000-8000-00000000000b")))
    q = store.get(answered(store))
    assert q.pane_id == "w1:p1" and q.delivery_error.startswith(service.NO_AGENT)


def test_two_panes_with_the_session_are_ambiguous_and_never_chosen(store, moved, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w1:p7", "aaaaaaaa-0000-4000-8000-00000000000a"), ("w1:p9", "aaaaaaaa-0000-4000-8000-00000000000a")))
    q = store.get(answered(store))
    assert q.pane_id == "w1:p1" and q.delivery_error.startswith(service.NO_AGENT)


def test_a_moved_and_continued_conversation_is_found(store, moved, monkeypatch, fake_herdr):
    continued()
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w1:p7", OLD)))  # herdr still says OLD
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", NEW)
    qid = answered(store, NEW)
    assert store.get(qid).pane_id == "w1:p7"
    worker.tick(store)
    assert store.get(qid).acknowledged_at


def test_an_answer_already_marked_gone_is_revived_when_its_agent_turns_up(store, moved, fake_herdr):
    qid = answered(store)
    store.db.execute("UPDATE questions SET pane_id = 'w1:p1', delivery_error = 'agent gone: no live agent in pane w1:p1'"
                     " WHERE id = ?", (qid,))
    store.mark_agent_gone(qid, "no live agent in pane w1:p1")
    worker.tick(store)
    worker.tick(store)
    q = store.get(qid)
    assert q.pane_id == "w1:p7" and not store.agent_gone(qid) and q.delivered_at


def test_a_pane_now_holding_another_conversation_sends_the_answer_after_its_own(store, fake_herdr, monkeypatch):
    """The pane id was reused by another conversation (CHANGED); ours runs in w1:p7."""
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", "aaaaaaaa-0000-4000-8000-00000000000a", "/r"), "Still me?")["id"]
    monkeypatch.setenv("FAKE_HERDR_SESSION", "dddddddd-0000-4000-8000-00000000000d")  # w1:p1 now runs another conversation
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w1:p1", "dddddddd-0000-4000-8000-00000000000d"), ("w1:p7", "aaaaaaaa-0000-4000-8000-00000000000a")))
    service.answer(store, qid, "yes")
    assert store.get(qid).pane_id == "w1:p7"
    assert not fake_herdr.calls_of("agent", "prompt")  # never typed into the other conversation


def test_another_herdr_servers_gone_rows_are_left_alone(store, moved, monkeypatch):
    qid = answered(store)
    store.db.execute("UPDATE questions SET pane_id = 'w1:p1' WHERE id = ?", (qid,))
    store.mark_agent_gone(qid, "x")
    store.db.execute("UPDATE question_ext SET herdr_server = '/other.sock' WHERE id = ?", (qid,))
    service.revive_moved(store)
    assert store.agent_gone(qid) and store.get(qid).pane_id == "w1:p1"



def gone(store, qid):
    store.db.execute("UPDATE questions SET pane_id = 'w1:p1', tab_label = 'old-tab',"
                     " delivery_error = 'agent gone: no live agent in pane w1:p1' WHERE id = ?", (qid,))
    store.mark_agent_gone(qid, "no live agent in pane w1:p1")


def test_42_a_hidden_answer_is_never_revived(store, moved, fake_herdr):
    """#42 Medium: the operator hid the failed delivery (dealt with it): never typed later."""
    qid = answered(store)
    gone(store, qid)
    store.hide_failed(qid)
    worker.tick(store)
    worker.tick(store)
    q = store.get(qid)
    assert q.pane_id == "w1:p1" and store.agent_gone(qid) and q.delivered_at is None
    assert not [c for c in fake_herdr.calls_of("agent", "prompt") if c[2] == "w1:p7"]


def test_42_a_dismissed_question_is_never_revived(store, moved):
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", "aaaaaaaa-0000-4000-8000-00000000000a", "/r"), "Dismissed?")["id"]
    store.mark_agent_gone(qid, "x")
    service.close(store, qid, "dismissed")
    service.revive_moved(store)
    assert store.get(qid).pane_id == "w1:p1"


def test_42_a_send_in_flight_is_not_revived_under_it(store, moved):
    qid = answered(store)
    gone(store, qid)
    store.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ?", (f"sending:{st.now()}:x", qid))
    service.revive_moved(store)
    assert store.get(qid).pane_id == "w1:p1"


def test_42_revival_backs_off_per_answer(store, moved, fake_herdr, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w1:p8", "bbbbbbbb-0000-4000-8000-00000000000b")))  # really gone
    qid = answered(store)
    gone(store, qid)
    fake_herdr.reset()
    service.revive_moved(store)
    service.revive_moved(store)
    assert len(fake_herdr.calls_of("agent", "list")) == 1  # the second call is within REVIVE_EVERY
    monkeypatch.setattr(service, "REVIVE_EVERY", 0.0)
    service.revive_moved(store)
    assert len(fake_herdr.calls_of("agent", "list")) == 2
