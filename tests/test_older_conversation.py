"""Live 2026-10-17: session 1cd70de7 continued in c5240607 (11:22:22Z); c5240607 asked Q129,
Q141, Q143, Q144, Q151, Q152. herdr later resumed the pane under the OLDER id 1cd70de7 (its
transcript still ended in `continued-in`), the inbox typed the answers there (06:32:56Z), and
the agent replied "they belong to another session's questions ... I'm leaving them alone".
The 0.10.5 reconcile then marked them delivered and verified, because the text was in
1cd70de7's transcript.

An ANCESTOR (a session that continued into the asker) is a different conversation, without
the asker's context. An answer goes to the asker or its forward continuations only; a pane
running an ancestor gets nothing typed and the row says why; a landing in an ancestor's
transcript is never a delivery; and the asker, once it runs again, gets it exactly once."""

import os
import time

import pytest

from herdr_inbox import herdr, hook, service, store as st, transcript, worker
from herdr_inbox.service import Caller

from .test_failed_pile import agents, continued_then_resumed, filed
from .test_session_continuation import NEW, OLD, continued, tdir, write


@pytest.fixture(autouse=True)
def _quick(monkeypatch):
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    getattr(service, "_reconciled_at", {}).clear()


def herdr_resumed_old(monkeypatch):
    """The captured order: OLD -> NEW, then herdr resumed OLD; our SessionStart hook fired for
    OLD (after the continued-in record), nothing typed there yet."""
    continued_then_resumed()
    hook_session_start(OLD, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", OLD)  # typed text would land in OLD


def hook_session_start(session, monkeypatch, source="resume"):
    import io
    import json

    monkeypatch.setenv("HERDR_PANE_ID", "w1:p1")
    monkeypatch.setattr("sys.stdin", io.StringIO(json.dumps(
        {"session_id": session, "source": source, "transcript_path": str(tdir() / f"{session}.jsonl")})))
    assert hook.run("session-start") == 0


def prompts(fake_herdr):
    return fake_herdr.calls_of("agent", "prompt")


def test_the_session_start_hook_marks_a_resumed_old_session_as_running_itself(store, fake_herdr, monkeypatch):
    continued_then_resumed()
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    assert herdr.live_agent("w1:p1")["session_id"] == NEW  # D6 alone: the window of the bug
    hook_session_start(OLD, monkeypatch)
    agent = herdr.live_agent("w1:p1")
    assert agent["session_id"] == OLD and agent["reported_session_id"] == OLD


def test_a_pane_running_an_older_session_gets_nothing_typed_and_says_why(store, fake_herdr, monkeypatch):
    herdr_resumed_old(monkeypatch)
    q = filed(store, "w1:p1", NEW, "Where should the field-encryption keys live?", "Secrets Manager")
    service.deliver(store, q)
    assert not prompts(fake_herdr)
    r = store.get(q.id)
    assert not r.delivered_at and not r.acknowledged_at and not r.verified_at
    assert r.delivery_error == (f"{service.OLDER}pane w1:p1 runs {OLD[:8]}, an older session than the asking "
                                f"{NEW[:8]}; resume {NEW[:8]} to deliver")
    assert [x.id for x in store.failed_deliveries()] == [q.id]  # the popup's ⚠ pile
    for _ in range(3):  # the worker keeps it (never 'agent gone') and never types it there
        worker.tick(store)
    r = store.get(q.id)
    assert not prompts(fake_herdr) and r.delivery_error.startswith(service.OLDER) and not store.agent_gone(q.id)


def test_send_now_never_types_into_an_older_session_either(store, fake_herdr, monkeypatch):
    herdr_resumed_old(monkeypatch)
    q = filed(store, "w1:p1", NEW, "Revoke and add redacted views?", "Revoke + views")
    service.deliver(store, q, retry=True, force=True)
    assert not prompts(fake_herdr) and store.get(q.id).delivery_error.startswith(service.OLDER)


def test_the_asker_resumed_later_gets_it_exactly_once(store, fake_herdr, monkeypatch):
    herdr_resumed_old(monkeypatch)
    q = filed(store, "w1:p1", NEW, "Rotate the gateway creds now?", "yes, tonight")
    service.deliver(store, q)
    assert not prompts(fake_herdr)
    # the operator resumes the asking session in that pane
    hook_session_start(NEW, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_SESSION", NEW)
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", NEW)
    for _ in range(3):
        worker.tick(store)
    assert len(prompts(fake_herdr)) == 1
    r = store.get(q.id)
    assert r.delivered_at and r.verified_at and r.acknowledged_at and r.delivery_error is None
    service.deliver(store, store.get(q.id), retry=True, force=True)
    assert len(prompts(fake_herdr)) == 1


def test_the_asker_resumed_in_another_pane_gets_it_there_once(store, fake_herdr, monkeypatch):
    herdr_resumed_old(monkeypatch)
    q = filed(store, "w1:p1", NEW, "Ship the redaction migration?", "after the views")
    service.deliver(store, q)
    assert store.get(q.id).delivery_error.startswith(service.OLDER)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w1:p1", OLD), ("w2:p9", NEW)))
    worker.tick(store)  # moved to the pane that runs the asker
    assert store.get(q.id).pane_id == "w2:p9"
    monkeypatch.setenv("FAKE_HERDR_SESSION", NEW)  # the fake's `agent get` for any pane
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", NEW)
    for _ in range(2):
        worker.tick(store)
    assert [c[2] for c in prompts(fake_herdr)] == ["w2:p9"]
    assert store.get(q.id).verified_at


def test_typed_into_an_unmarked_older_session_is_recorded_and_never_typed_again(store, fake_herdr, monkeypatch):
    """No SessionStart marker (an older plugin, or the hook did not run): the inbox cannot tell
    before typing. It types once, finds the text in the older session only, and says so; then
    the older session's own transcript shows it runs itself, and nothing more is typed there."""
    continued_then_resumed()
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", OLD)
    q = filed(store, "w1:p1", NEW, "Which KMS key alias?", "alias/gw-creds")
    service.deliver(store, q)
    assert len(prompts(fake_herdr)) == 1
    r = store.get(q.id)
    assert not r.verified_at and not r.acknowledged_at and not r.delivered_at
    assert r.delivery_error.startswith(f"{service.OLDER}typed into {OLD[:8]} in pane w1:p1")
    for _ in range(3):
        worker.tick(store)
    assert len(prompts(fake_herdr)) == 1
    assert store.get(q.id).delivery_error.startswith(service.OLDER)


def test_the_detached_verify_job_never_counts_an_older_landing(store, fake_herdr, monkeypatch):
    continued_then_resumed()
    q = filed(store, "w1:p1", NEW, "Detached verify?", "fine")
    claim = store.claim_delivery(q.id)
    text = herdr.clean(service.delivery_text([store.get(q.id)]), limit=10**9)
    new_path, old_path = tdir() / f"{NEW}.jsonl", tdir() / f"{OLD}.jsonl"
    job = {"path": str(new_path), "offset": transcript.size(new_path), "ack": True, "busy": False, "text": text,
           "watch": [[str(new_path), transcript.size(new_path)]], "older": [[str(old_path), transcript.size(old_path)]],
           "rows": [{"id": q.id, "claim": claim, "older_error": service.older_error("w1:p1", OLD, NEW, typed=True)}]}
    write(OLD, [{"type": "user", "timestamp": st.now(), "message": {"role": "user", "content": text}}])
    service.verify_job(store, job)
    r = store.get(q.id)
    assert not r.verified_at and r.delivery_error.startswith(f"{service.OLDER}typed into {OLD[:8]}")


def test_a_forward_continuation_running_in_the_pane_is_delivered_once(store, fake_herdr, monkeypatch):
    """The other direction stays as it was: OLD asked, its process continued in NEW (herdr and
    the env keep reporting OLD). OLD's own start marker predates the continuation."""
    q = filed(store, "w1:p1", OLD, "Asked before the continuation?", "yes")
    continued()
    hook_session_start(OLD, monkeypatch, source="startup")
    marker = st._starts_dir() / OLD
    os.utime(marker, (time.time() - 10**8, time.time() - 10**8))  # started long before the continued-in
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", NEW)
    assert herdr.live_agent("w1:p1")["session_id"] == NEW
    service.deliver(store, store.get(q.id))
    worker.tick(store)
    assert len(prompts(fake_herdr)) == 1
    r = store.get(q.id)
    assert r.verified_at and r.acknowledged_at and r.delivery_error is None


def test_an_answer_for_an_older_asker_reaches_its_forward_continuation(store, fake_herdr, monkeypatch):
    """OLD asked, continued in NEW, then OLD was resumed elsewhere (its file goes on): NEW
    still carries OLD's conversation, so a pane running NEW takes OLD's answer."""
    q = filed(store, "w1:p1", OLD, "Asked by the older session?", "noted")
    continued()
    write(OLD, [{"type": "user", "timestamp": st.now(), "message": {"role": "user", "content": "resumed elsewhere"}}])
    monkeypatch.setenv("FAKE_HERDR_SESSION", NEW)
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", NEW)
    service.deliver(store, store.get(q.id))
    assert len(prompts(fake_herdr)) == 1 and store.get(q.id).verified_at


def test_the_cli_answer_from_a_resumed_older_session_is_not_the_askers_own(store, fake_herdr, monkeypatch):
    """#44 treated `inbox answer` from the resumed OLD as the asker's own (Q159-Q166 shape): the
    asking NEW never got it. Now it is delivered like any other answer, to the asker."""
    from herdr_inbox.cli import main

    continued_then_resumed()
    write(OLD, [{"type": "user", "timestamp": st.now(), "message": {"role": "user", "content": "back"}}])
    store.record_session(OLD, str(tdir() / f"{OLD}.jsonl"))  # its SessionStart hook ran
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", NEW, "/r"), "Relay the operator's call?")["id"]
    monkeypatch.setenv("CLAUDE_CODE_SESSION_ID", OLD)
    monkeypatch.setenv("FAKE_HERDR_SESSION", NEW)  # the asker runs in its pane
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", NEW)
    assert main(["answer", str(qid), "Keep as built, decided in chat"]) == 0
    assert len(prompts(fake_herdr)) == 1
    q = store.get(qid)
    assert q.verified_at and q.acknowledged_at


def test_a_hook_in_a_resumed_older_session_never_carries_the_askers_answer(store, fake_herdr, monkeypatch):
    herdr_resumed_old(monkeypatch)
    q = filed(store, "w1:p1", NEW, "Hook path?", "no")
    assert hook.deliver({"session_id": OLD, "transcript_path": str(tdir() / f"{OLD}.jsonl")}) is None
    assert not store.get(q.id).delivered_at


def test_older_conversation_is_directional(store, fake_herdr):
    continued()
    assert service.older_conversation(store, OLD, NEW)
    assert not service.older_conversation(store, NEW, OLD)
    assert not service.older_conversation(store, NEW, NEW)
    assert service.forward_family(store, NEW) == [NEW]
    assert service.forward_family(store, OLD) == [OLD, NEW]
