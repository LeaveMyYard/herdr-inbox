"""Edges of deliver_batch's decision that the rest of the suite never reaches (rewrite plan
§5.2 item 5): each test asserts Python's behaviour at one branch, and the decision recorder
turns it into a row of tests/fixtures/decisions.json that the Rust port
(crates/inbox-core/tests/deliver_rows.rs) must reproduce."""

import json
import os
from pathlib import Path

import pytest

from herdr_inbox import herdr, service, worker
from herdr_inbox.service import Caller
from herdr_inbox.store import REQUEUED

from .test_failed_pile import continued_then_resumed, filed
from .test_session_continuation import NEW, OLD

ASKER = "aaaaaaaa-0000-4000-8000-00000000000a"
A = Caller("w1:p1", "w1:t1", "tab-A", ASKER, "/r")


@pytest.fixture(autouse=True)
def _quick(monkeypatch):
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)


@pytest.fixture
def live_session(fake_herdr):
    path = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-fake-project" / f"{ASKER}.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"type": "user", "message": {"content": "hello"}}) + "\n")
    return path


def prompts(fake):
    return fake.calls_of("agent", "prompt")


def waiting_answer(store, monkeypatch, caller=A):
    """An answered question held pending (its agent was at a dialog when it was answered)."""
    qid = service.ask(store, caller, "Merge it?", options=["yes", "no"])["id"]
    monkeypatch.setenv("FAKE_HERDR_STATUS", "blocked")
    service.answer(store, qid, "yes")
    assert store.get(qid).delivery_error.startswith("pending: blocked")
    return qid


def misses(store, qid, n):
    store.db.execute("INSERT INTO question_ext (id, group_id, hook_misses) VALUES (?, ?, ?)"
                     " ON CONFLICT(id) DO UPDATE SET hook_misses = excluded.hook_misses", (qid, qid, n))


def test_an_answer_without_a_pane_waits_for_check_answers(store, fake_herdr):
    """``ask`` refuses a caller outside herdr; a row without a pane is an older version's."""
    qid = service.ask(store, A, "Merge it?")["id"]
    store.db.execute("UPDATE questions SET pane_id = NULL WHERE id = ?", (qid,))
    service.answer(store, qid, "yes")
    assert store.get(qid).delivery_error == "no pane: answer available through check_answers"
    assert prompts(fake_herdr) == []


@pytest.mark.parametrize("n", [1, 2])
def test_a_watched_idle_session_keeps_answers_its_hooks_missed_once(store, fake_herdr, monkeypatch, live_session, n):
    """Under MAX_HOOK_MISSES the watcher has it; at the cap the answer is typed."""
    qid = waiting_answer(store, monkeypatch)
    misses(store, qid, n)
    monkeypatch.setattr(service, "_watched", lambda store, session: True)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    service.deliver_batch(store, [store.get(qid)], retry=True)
    if n < service.MAX_HOOK_MISSES:
        assert store.get(qid).delivery_error == f"pending: {service.WATCHER_WAIT}" and prompts(fake_herdr) == []
    else:
        assert len(prompts(fake_herdr)) == 1 and store.get(qid).verified_at


@pytest.mark.parametrize("n", [1, 2])
def test_a_working_session_with_a_live_hook_keeps_answers_it_missed_once(store, fake_herdr, monkeypatch, live_session, n):
    qid = waiting_answer(store, monkeypatch)
    misses(store, qid, n)
    monkeypatch.setattr(type(store), "hook_live", lambda self, session, within: True)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.deliver_batch(store, [store.get(qid)], retry=True)
    error = store.get(qid).delivery_error
    if n < service.MAX_HOOK_MISSES:
        assert error == f"pending: {service.HOOK_WAIT}"
    else:  # left to typing: the busy grace applies
        assert error == "pending: busy: the agent is working"
    assert prompts(fake_herdr) == []


@pytest.mark.parametrize("force", [False, True])
def test_a_requeued_answer_waits_for_rest_unless_sent_now(store, fake_herdr, monkeypatch, live_session, force):
    qid = waiting_answer(store, monkeypatch)
    store.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ?", (REQUEUED, qid))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.deliver_batch(store, [store.get(qid)], retry=not force, force=force)
    if force:
        assert len(prompts(fake_herdr)) == 1 and store.get(qid).verified_at
    else:
        assert store.get(qid).delivery_error == REQUEUED and prompts(fake_herdr) == []


def test_send_now_from_the_retry_path_types_into_a_focused_pane(store, fake_herdr, monkeypatch, live_session):
    """``deliver(retry=True, force=True)``: the operator's own "Send now" skips both focus checks."""
    qid = waiting_answer(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "1")
    service.deliver(store, store.get(qid), retry=True, force=True)
    assert len(prompts(fake_herdr)) == 1 and store.get(qid).verified_at


def test_an_agent_gone_right_before_typing_gets_nothing(store, fake_herdr, monkeypatch, live_session):
    qid = waiting_answer(store, monkeypatch)
    real, seen = herdr.live_agent, {"n": 0}

    def live_agent(pane):
        seen["n"] += 1
        return real(pane) if seen["n"] == 1 else None  # gone between the checks

    monkeypatch.setattr(herdr, "live_agent", live_agent)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    service.deliver_batch(store, [store.get(qid)], retry=True)
    assert store.get(qid).delivery_error == "pending: waiting: the operator is in that pane"
    assert prompts(fake_herdr) == []


def test_text_lost_while_an_older_session_was_watched_is_not_verified(store, fake_herdr, monkeypatch):
    """The pane runs an older session (no marker); the typed text lands nowhere: an ordinary
    unverified attempt, never an older-conversation landing."""
    continued_then_resumed()
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    q = filed(store, "w1:p1", NEW, "Which KMS key alias?", "alias/gw-creds")
    service.deliver(store, q)
    assert len(prompts(fake_herdr)) == 1
    assert store.get(q.id).delivery_error == service.NOT_VERIFIED
    worker.tick(store)  # the retry loop types it again: still the ordinary cap


def test_a_retry_never_types_into_a_conversation_that_replaced_the_asker(store, fake_herdr, monkeypatch, live_session):
    """The pane switched sessions between the first look and the prompt (#92 review M1)."""
    qid = waiting_answer(store, monkeypatch)
    real, seen = herdr.live_agent, {"n": 0}

    def live_agent(pane):
        seen["n"] += 1
        agent = real(pane)
        return agent if seen["n"] == 1 else {**agent, "session_id": "cccccccc-0000-4000-8000-00000000000c"}

    monkeypatch.setattr(herdr, "live_agent", live_agent)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    service.deliver_batch(store, [store.get(qid)], retry=True)
    assert store.get(qid).delivery_error == "pending: waiting: the operator is in that pane"
    assert prompts(fake_herdr) == []


def test_a_busy_status_other_than_working_gets_the_busy_grace_not_the_hook(store, fake_herdr, monkeypatch, live_session):
    """Any status outside idle/done (and not blocked) is busy; only `working` is hook-carried
    (#92 review L2)."""
    qid = waiting_answer(store, monkeypatch)
    monkeypatch.setattr(type(store), "hook_live", lambda self, session, within: True)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "unknown")
    service.deliver_batch(store, [store.get(qid)], retry=True)
    assert store.get(qid).delivery_error == "pending: busy: the agent is unknown"
    assert prompts(fake_herdr) == []


def test_text_typed_into_an_unknown_status_agent_and_lost_is_a_busy_miss(store, fake_herdr, monkeypatch, live_session):
    """Past the busy grace an `unknown` agent is typed into like a working one; text it never
    showed counts against the busy cap, not the idle one (#92 review L2)."""
    from datetime import timedelta

    from herdr_inbox.store import now

    qid = waiting_answer(store, monkeypatch)
    store.db.execute("INSERT INTO question_ext (id, group_id, pending_since) VALUES (?, ?, ?)"
                     " ON CONFLICT(id) DO UPDATE SET pending_since = excluded.pending_since",
                     (qid, qid, now(-timedelta(seconds=service.BUSY_GRACE + 1))))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "unknown")
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    service.deliver_batch(store, [store.get(qid)], retry=True)
    assert len(prompts(fake_herdr)) == 1
    q = store.get(qid)
    assert q.delivery_error == f"pending: {service.BUSY_WAIT_IDLE}" and q.delivery_attempts == 0
    assert store.busy_attempts(qid) == 1


# -- several answers in one pane (#92 final pass M1) ------------------------------------------


def waiting_answers(store, monkeypatch, *answers):
    """Answers to questions of one pane, all held pending (the agent was at a dialog)."""
    monkeypatch.setenv("FAKE_HERDR_STATUS", "blocked")
    ids = []
    for i, answer in enumerate(answers):
        qid = service.ask(store, A, f"Question {i + 1}?")["id"]
        service.answer(store, qid, answer)
        ids.append(qid)
    return ids


def pass_of(store, ids, **kw):
    service.deliver_batch(store, [store.get(i) for i in ids], **kw)
    return [store.get(i) for i in ids]


def test_a_draft_in_the_input_box_holds_back_every_answer_of_the_message(store, fake_herdr, monkeypatch, live_session):
    from .test_composer_draft import screen

    ids = waiting_answers(store, monkeypatch, "yes", "no", "maybe")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", screen("draft"))
    qs = pass_of(store, ids, retry=True)
    assert prompts(fake_herdr) == []
    assert all(q.delivery_error == f"pending: {service.COMPOSER_DRAFT}" for q in qs)


def test_a_failed_prompt_fails_every_answer_of_the_message(store, fake_herdr, monkeypatch, live_session):
    ids = waiting_answers(store, monkeypatch, "yes", "no")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    qs = pass_of(store, ids, retry=True)
    assert all("fake failure" in q.delivery_error and q.delivery_attempts == 1 for q in qs)


def test_only_the_answer_not_yet_in_the_conversation_is_typed(store, fake_herdr, monkeypatch, live_session):
    first, second = waiting_answers(store, monkeypatch, "yes", "no")
    with open(live_session, "a") as fh:  # the second one landed late, after its pass gave up
        fh.write(json.dumps({"type": "user", "message": {"content": service.delivery_text([store.get(second)])}}) + "\n")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    q1, q2 = pass_of(store, [first, second], retry=True)
    assert len(prompts(fake_herdr)) == 1 and service.delivery_text([q1]) in prompts(fake_herdr)[0][3]
    assert q1.verified_at and q2.verified_at and q2.acknowledged_at


def test_a_long_answer_goes_in_a_message_of_its_own_as_a_reference(store, fake_herdr, monkeypatch, live_session):
    short, long = waiting_answers(store, monkeypatch, "yes", "x" * 5000)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    q1, q2 = pass_of(store, [short, long], retry=True)
    assert len(prompts(fake_herdr)) == 2
    assert q1.acknowledged_at and q2.verified_at and not q2.acknowledged_at


def test_a_busy_answer_typed_into_an_older_session_is_recorded_as_such_not_retried(store, fake_herdr, monkeypatch):
    """Busy (Send now into a working agent) and the text landed only in the older session the
    pane runs: the older-conversation state, never a busy miss to type again."""
    continued_then_resumed()
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", OLD)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    q = filed(store, "w1:p1", NEW, "Which KMS key alias?", "alias/gw-creds")
    service.deliver(store, q, force=True)
    r = store.get(q.id)
    assert len(prompts(fake_herdr)) == 1 and store.busy_attempts(q.id) == 0
    assert r.delivery_error.startswith(f"{service.OLDER}typed into {OLD[:8]}")
