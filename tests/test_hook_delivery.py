"""0.9.0: answers reach a BUSY Claude session through its own hooks (PostToolBatch / Stop
additionalContext), not typing. See DESIGN.md, "Hook delivery to busy Claude sessions"."""

import json

import pytest

from herdr_inbox import service, store as st, transcript, worker
from herdr_inbox.service import Caller

from .test_hook import run_hook

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")


@pytest.fixture
def tpath(tmp_path):
    p = tmp_path / "sess-A.jsonl"
    p.write_text("")
    return p


def answered_busy(store, monkeypatch, question="Merge it?", answer="squash"):
    """An answer the worker could not type: the agent was working (pending, busy grace)."""
    qid = service.ask(store, A, question)["id"]
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.answer(store, qid, answer)
    q = store.get(qid)
    assert q.delivered_at is None and (q.delivery_error or "").startswith("pending: busy")
    return qid


def payload(tpath, **extra):
    return {"session_id": "sess-A", "transcript_path": str(tpath), **extra}


def context(proc, event):
    assert proc.returncode == 0, proc.stderr
    if not proc.stdout.strip():
        return None
    out = json.loads(proc.stdout)["hookSpecificOutput"]
    assert out["hookEventName"] == event
    return out["additionalContext"]


def hook_record(text, event="PostToolBatch"):
    return json.dumps({"type": "attachment", "timestamp": st.now(),
                       "attachment": {"type": "hook_additional_context", "content": [text], "hookEvent": event}})


@pytest.mark.parametrize("event,name", [("post-tool-batch", "PostToolBatch"), ("stop", "Stop")])
def test_a_busy_sessions_answer_goes_out_as_hook_context_once(store, fake_herdr, monkeypatch, tpath, event, name):
    qid = answered_busy(store, monkeypatch)
    ctx = context(run_hook(event, payload(tpath)), name)
    assert ctx == f"[inbox] Q{qid} answered — Merge it?: squash"
    q = store.get(qid)
    assert q.delivered_at and q.acknowledged_at is None  # sent; seen only once the transcript shows it
    assert store.db.execute("SELECT hook_sent_at FROM question_ext WHERE id = ?", (qid,)).fetchone()[0]
    assert context(run_hook(event, payload(tpath)), name) is None  # exactly once
    assert not fake_herdr.calls_of("agent", "prompt")  # nothing typed


def test_several_answers_go_in_one_message(store, fake_herdr, monkeypatch, tpath):
    q1 = answered_busy(store, monkeypatch, "One?", "a")
    q2 = answered_busy(store, monkeypatch, "Two?", "b")
    ctx = context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
    assert ctx == f"[inbox] answered — Q{q1} One?: a · Q{q2} Two?: b"


def test_nothing_for_a_subagent_another_session_or_outside_herdr(store, fake_herdr, monkeypatch, tpath):
    qid = answered_busy(store, monkeypatch)
    assert context(run_hook("post-tool-batch", payload(tpath, agent_id="agent-1")), "PostToolBatch") is None
    assert context(run_hook("post-tool-batch", {**payload(tpath), "session_id": "sess-B"}), "PostToolBatch") is None
    assert context(run_hook("post-tool-batch", payload(tpath), HERDR_PANE_ID=""), "PostToolBatch") is None
    assert store.get(qid).delivered_at is None


def test_a_row_held_by_another_sender_or_already_seen_is_left_alone(store, fake_herdr, monkeypatch, tpath):
    qid = answered_busy(store, monkeypatch)
    store.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ?", (f"sending:{st.now()}:other", qid))
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch") is None
    store.db.execute("UPDATE questions SET delivery_error = NULL, acknowledged_at = ? WHERE id = ?", (st.now(), qid))
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch") is None


def test_an_answer_already_in_the_transcript_is_recorded_not_resent(store, fake_herdr, monkeypatch, tpath):
    qid = answered_busy(store, monkeypatch)
    tpath.write_text(json.dumps({"type": "user", "timestamp": st.now(),
                                 "message": {"content": f"[inbox] Q{qid} answered — Merge it?: squash"}}) + "\n")
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch") is None
    q = store.get(qid)
    assert q.delivered_at and q.acknowledged_at  # a late typed landing: verified


def test_the_matcher_reads_hook_context_records(tpath):
    tpath.write_text(hook_record("[inbox] Q7 answered — X?: y") + "\n")
    assert transcript.landed_since(tpath, 0, [7])
    assert transcript.landed_ids(tpath, [7, 8]) == {7}


def test_the_worker_verifies_a_hook_delivery(store, fake_herdr, monkeypatch, tpath):
    qid = answered_busy(store, monkeypatch)
    ctx = context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
    tpath.write_text(hook_record(ctx) + "\n")
    worker.tick(store)
    q = store.get(qid)
    assert q.acknowledged_at and store.db.execute("SELECT verified_at FROM question_ext WHERE id = ?",
                                                  (qid,)).fetchone()[0]


def test_an_unseen_hook_delivery_goes_back_to_pending_then_to_typing(store, fake_herdr, monkeypatch, tpath):
    monkeypatch.setattr(service, "HOOK_VERIFY_SECONDS", 0.0)
    qid = answered_busy(store, monkeypatch)
    for miss in (1, 2):
        assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
        worker.tick(store)  # never in the transcript
        q = store.get(qid)
        assert q.delivered_at is None and q.acknowledged_at is None
        assert store.db.execute("SELECT hook_misses FROM question_ext WHERE id = ?", (qid,)).fetchone()[0] == miss
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch") is None  # typing's turn now


def test_session_start_marks_the_session_hook_capable(store, fake_herdr, tpath):
    run_hook("session-start", {**payload(tpath), "source": "startup"})
    assert store.db.execute("SELECT hook_delivery FROM sessions WHERE session_id = 'sess-A'").fetchone()[0]


def test_the_worker_never_types_into_a_working_hook_capable_session(store, fake_herdr, monkeypatch, tpath):
    run_hook("session-start", {**payload(tpath), "source": "startup"})
    qid = answered_busy(store, monkeypatch)
    store.db.execute("UPDATE question_ext SET pending_since = ? WHERE id = ?",
                     (st.now(-st.timedelta(minutes=5)), qid))  # long past the busy grace
    monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-A")
    worker.tick(store)
    assert not fake_herdr.calls_of("agent", "prompt")
    assert "next tool call or turn end" in store.get(qid).delivery_error
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")  # at rest: typing, as before
    worker.tick(store)
    assert fake_herdr.calls_of("agent", "prompt")


def test_a_session_without_the_hook_keeps_the_busy_grace_typing(store, fake_herdr, monkeypatch):
    qid = answered_busy(store, monkeypatch)
    store.db.execute("UPDATE question_ext SET pending_since = ? WHERE id = ?",
                     (st.now(-st.timedelta(minutes=5)), qid))
    monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-A")
    worker.tick(store)
    assert fake_herdr.calls_of("agent", "prompt")  # 0.8 behaviour for sessions without the hook


@pytest.mark.parametrize("event,name", [("PostToolBatch", "PostToolBatch"), ("Stop", "Stop")])
def test_the_registered_hook_delivers_through_the_stdlib_launcher(store, fake_herdr, monkeypatch, tpath, event, name):
    """hooks.json as Claude Code runs it: sh -> python3 -I hooks/run.py (no uv, no venv)."""
    from .test_hook_launch import ROOT, run_hook as launch

    qid = answered_busy(store, monkeypatch)
    proc = launch(event, ROOT, tpath.parent / "data", payload=payload(tpath))
    assert proc.returncode == 0, proc.stderr
    out = json.loads(proc.stdout)["hookSpecificOutput"]
    assert out == {"hookEventName": name, "additionalContext": f"[inbox] Q{qid} answered — Merge it?: squash"}


def test_a_session_without_answers_costs_one_read_only_query(store, fake_herdr, tpath):
    from .test_hook_launch import ROOT, run_hook as launch

    proc = launch("PostToolBatch", ROOT, tpath.parent / "data", payload=payload(tpath))
    assert proc.returncode == 0 and proc.stdout == ""
    assert store.db.execute("SELECT COUNT(*) FROM sessions").fetchone()[0] == 0  # nothing written
