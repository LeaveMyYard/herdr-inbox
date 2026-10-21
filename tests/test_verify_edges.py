"""Edges of the delivery verifications that the rest of the suite never reaches (rewrite plan
§5.2 item 5): each test asserts Python's behaviour at one branch, and the decision recorder
turns it into a verify_hook or verify_job row of tests/fixtures/decisions.json that the Rust
port (crates/inbox-core/tests/verify_rows.rs) must reproduce."""

import json
import os
from pathlib import Path

import pytest

from herdr_inbox import service, worker
from herdr_inbox.service import Caller

from .test_hook import run_hook
from .test_hook_delivery import answered_busy, context, hook_record, payload

ASKER = "aaaaaaaa-0000-4000-8000-00000000000a"
A = Caller("w1:p1", "w1:t1", "tab-A", ASKER, "/r")


@pytest.fixture
def tpath(tmp_path):
    p = tmp_path / f"{ASKER}.jsonl"
    p.write_text("")
    return p


def hook_sent(store):
    return store.db.execute("SELECT hook_sent_at FROM question_ext WHERE hook_sent_at IS NOT NULL").fetchall()


# -- verify_hook ------------------------------------------------------------------------------


def test_a_fresh_hook_send_not_yet_seen_is_left_for_the_next_tick(store, fake_herdr, monkeypatch, tpath):
    qid = answered_busy(store, monkeypatch)
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
    worker.tick(store)  # within HOOK_VERIFY_SECONDS: nothing decided
    q = store.get(qid)
    assert q.delivered_at and not q.acknowledged_at and hook_sent(store)
    assert store.hook_misses(qid) == 0


def test_a_hook_send_without_a_transcript_stays_sent_and_unseen(store, fake_herdr, monkeypatch, tmp_path):
    """No transcript to look in when the time is up: sent, never counted as seen (like typing
    without one), and no miss counted."""
    monkeypatch.setattr(service, "HOOK_VERIFY_SECONDS", 0.0)
    qid = answered_busy(store, monkeypatch)
    gone = tmp_path / "elsewhere" / f"{ASKER}.jsonl"  # never written
    assert context(run_hook("post-tool-batch", payload(gone)), "PostToolBatch")
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path / "no-config"))
    worker.tick(store)
    q = store.get(qid)
    assert q.delivered_at and not q.acknowledged_at and not hook_sent(store)
    assert store.hook_misses(qid) == 0


def test_a_long_answer_whose_reference_landed_is_verified_but_unseen(store, fake_herdr, monkeypatch, tpath):
    qid = answered_busy(store, monkeypatch, answer="x" * 5000)
    ctx = context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
    assert "check_answers" in ctx and "x" * 5000 not in ctx  # the reference, not the answer
    tpath.write_text(hook_record(ctx) + "\n")
    worker.tick(store)
    q = store.get(qid)
    assert not q.acknowledged_at
    assert store.db.execute("SELECT verified_at FROM question_ext WHERE id = ?", (qid,)).fetchone()[0]


# -- verify_job -------------------------------------------------------------------------------


@pytest.fixture
def live_session(fake_herdr):
    path = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-fake-project" / f"{ASKER}.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"type": "user", "message": {"content": "hello"}}) + "\n")
    return path


def typed_job(store, live_session, monkeypatch, lands, **job):
    """A claimed answer, typed by a popup whose `inbox verify` job then runs."""
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.2)
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    service.answer(store, qid, "yes", deliver=False)
    claim = store.claim_delivery(qid)
    offset = live_session.stat().st_size
    if lands:
        with open(live_session, "a") as fh:
            fh.write(json.dumps({"type": "user", "message": {"content": service.delivery_text([store.get(qid)])}}) + "\n")
    service.verify_job(store, {"path": str(live_session), "offset": offset, "rows": [{"id": qid, "claim": claim}], **job})
    return store.get(qid)


def test_a_typed_answer_never_seen_at_rest_is_not_verified(store, live_session, monkeypatch):
    q = typed_job(store, live_session, monkeypatch, lands=False, ack=True, busy=False)
    assert q.delivery_error == service.NOT_VERIFIED and q.delivery_attempts == 1


def test_a_typed_answer_never_seen_while_busy_waits_for_idle(store, live_session, monkeypatch):
    q = typed_job(store, live_session, monkeypatch, lands=False, ack=True, busy=True)
    assert q.delivery_error == f"pending: {service.BUSY_WAIT_IDLE}" and q.delivery_attempts == 0
    assert store.busy_attempts(q.id) == 1


def test_a_typed_reference_that_landed_is_delivered_but_unseen(store, live_session, monkeypatch):
    q = typed_job(store, live_session, monkeypatch, lands=True, ack=False, busy=False)
    assert q.delivered_at and q.verified_at and not q.acknowledged_at


def test_an_answer_whose_text_and_reference_both_landed_is_seen(store, fake_herdr, monkeypatch, tpath):
    """The reference alone keeps an answer unseen; with the answer itself there too it is seen
    (``landed - {REFERENCE_INDEX}`` not empty)."""
    qid = answered_busy(store, monkeypatch)
    ctx = context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
    reference = service._reference(store.get(qid))
    tpath.write_text(hook_record(ctx) + "\n" + hook_record(reference) + "\n")
    worker.tick(store)
    q = store.get(qid)
    assert q.acknowledged_at
    assert store.db.execute("SELECT verified_at FROM question_ext WHERE id = ?", (qid,)).fetchone()[0]


# -- a verify job with several rows (#93 review M1) -------------------------------------------


def claimed_answers(store, n):
    ids = []
    for i in range(n):
        qid = service.ask(store, A, f"Question {i + 1}?", options=["yes"])["id"]
        service.answer(store, qid, "yes", deliver=False)
        ids.append((qid, store.claim_delivery(qid)))
    return ids


def test_a_job_judges_every_row_by_its_own_plan(store, live_session, monkeypatch, tmp_path):
    """The typed text landed in the asker's transcript: a row counting it is delivered, a row
    whose plan counts only the other (older) session's transcript is that older state, and an
    older popup's plan-less row counts every watched transcript."""
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.2)
    other = live_session.parent / "bbbbbbbb-0000-4000-8000-00000000000b.jsonl"
    other.write_text("")
    (q1, c1), (q2, c2), (q3, c3) = claimed_answers(store, 3)
    text = service.delivery_text([store.get(q) for q in (q1, q2, q3)])
    offset = live_session.stat().st_size
    with open(live_session, "a") as fh:
        fh.write(json.dumps({"type": "user", "message": {"content": text}}) + "\n")
    older = service.older_error("w1:p1", "bbbbbbbb-0000-4000-8000-00000000000b", ASKER, typed=True)
    service.verify_job(store, {
        "path": str(live_session), "offset": offset, "ack": True, "busy": False, "text": text,
        "watch": [[str(live_session), offset], [str(other), 0]],
        "rows": [{"id": q1, "claim": c1, "counting": [str(live_session)], "older_error": None},
                 {"id": q2, "claim": c2},
                 {"id": q3, "claim": c3, "counting": [str(other)], "older_error": older}]})
    r1, r2, r3 = (store.get(q) for q in (q1, q2, q3))
    assert r1.verified_at and r1.acknowledged_at
    assert r2.verified_at and r2.acknowledged_at
    assert r3.delivery_error == older and not r3.verified_at


def test_a_job_whose_text_never_showed_misses_every_row(store, live_session, monkeypatch):
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.2)
    (q1, c1), (q2, c2) = claimed_answers(store, 2)
    offset = live_session.stat().st_size
    service.verify_job(store, {"path": str(live_session), "offset": offset, "busy": True,
                               "rows": [{"id": q1, "claim": c1, "counting": [str(live_session)]},
                                        {"id": q2, "claim": c2}]})
    for q in (q1, q2):
        assert store.get(q).delivery_error == f"pending: {service.BUSY_WAIT_IDLE}" and store.busy_attempts(q) == 1
