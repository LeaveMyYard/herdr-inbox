"""0.9.0: answers reach a BUSY Claude session through its own hooks (PostToolBatch / Stop
additionalContext), not typing. See DESIGN.md, "Hook delivery to busy Claude sessions"."""

import json

import pytest

from herdr_inbox import service, store as st, transcript, worker
from herdr_inbox.service import Caller

from .test_hook import run_hook

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")


@pytest.fixture
def tpath(tmp_path):
    p = tmp_path / "aaaaaaaa-0000-4000-8000-00000000000a.jsonl"
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
    return {"session_id": "aaaaaaaa-0000-4000-8000-00000000000a", "transcript_path": str(tpath), **extra}


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
    assert context(run_hook("post-tool-batch", {**payload(tpath), "session_id": "bbbbbbbb-0000-4000-8000-00000000000b"}), "PostToolBatch") is None
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


def seen_at(store):
    row = store.db.execute("SELECT hook_seen_at FROM sessions WHERE session_id = 'aaaaaaaa-0000-4000-8000-00000000000a'").fetchone()
    return row[0] if row else None


def test_only_the_delivery_hook_firing_marks_the_session_live_throttled(store, fake_herdr, tpath):
    run_hook("session-start", {**payload(tpath), "source": "startup"})
    assert seen_at(store) is None  # SessionStart says nothing about the delivery hooks firing (#26 M1)
    run_hook("post-tool-batch", payload(tpath))
    first = seen_at(store)
    assert first
    store.db.execute("UPDATE sessions SET hook_seen_at = ? WHERE session_id = 'aaaaaaaa-0000-4000-8000-00000000000a'", (st.now(-st.timedelta(seconds=30)),))
    run_hook("post-tool-batch", payload(tpath))
    assert seen_at(store) < first  # within the minute: no write
    store.db.execute("UPDATE sessions SET hook_seen_at = ? WHERE session_id = 'aaaaaaaa-0000-4000-8000-00000000000a'", (st.now(-st.timedelta(minutes=2)),))
    run_hook("stop", payload(tpath))
    assert seen_at(store) >= first


def past_grace(store, qid):
    store.db.execute("UPDATE question_ext SET pending_since = ? WHERE id = ?", (st.now(-st.timedelta(minutes=5)), qid))


def test_the_worker_never_types_into_a_working_session_whose_hook_is_live(store, fake_herdr, monkeypatch, tpath):
    run_hook("post-tool-batch", payload(tpath))  # the hook fired (nothing to deliver yet)
    qid = answered_busy(store, monkeypatch)
    past_grace(store, qid)
    monkeypatch.setenv("FAKE_HERDR_SESSION", "aaaaaaaa-0000-4000-8000-00000000000a")
    worker.tick(store)
    assert not fake_herdr.calls_of("agent", "prompt")
    assert "next tool call or turn end" in store.get(qid).delivery_error
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")  # at rest: typing, as before
    worker.tick(store)
    assert fake_herdr.calls_of("agent", "prompt")


@pytest.mark.parametrize("fired", [False, True])
def test_a_session_whose_hook_never_fires_or_went_quiet_gets_0_8_busy_typing(store, fake_herdr, monkeypatch, tpath, fired):
    """#26 M1: started with 0.9 (SessionStart ran) but the delivery hook never fires (plugin
    updated or disabled mid-session), or fired long ago: answers are not parked forever."""
    run_hook("session-start", {**payload(tpath), "source": "startup"})
    if fired:
        run_hook("post-tool-batch", payload(tpath))
        store.db.execute("UPDATE sessions SET hook_seen_at = ? WHERE session_id = 'aaaaaaaa-0000-4000-8000-00000000000a'",
                         (st.now(-service.HOOK_LIVE - st.timedelta(minutes=1)),))
    qid = answered_busy(store, monkeypatch)
    past_grace(store, qid)
    monkeypatch.setenv("FAKE_HERDR_SESSION", "aaaaaaaa-0000-4000-8000-00000000000a")
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    worker.tick(store)
    assert fake_herdr.calls_of("agent", "prompt")  # the 0.8 busy grace, then typing


def test_a_stale_verifier_read_never_wipes_a_newer_send(store, fake_herdr, monkeypatch, tpath):
    """#26 L1: compare-and-set on the hook_sent_at that was judged."""
    qid = answered_busy(store, monkeypatch)
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
    (_, old_sent), = store.hook_unverified(service.herdr.server_id())
    assert store.hook_unseen(qid, old_sent, service.HOOK_UNSEEN)  # first miss: re-pended
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")  # a fresh send
    store.db.execute("UPDATE question_ext SET hook_sent_at = ? WHERE id = ?", (st.now(st.timedelta(seconds=1)), qid))
    assert not store.hook_unseen(qid, old_sent, service.HOOK_UNSEEN)  # the stale judgement
    q = store.get(qid)
    assert q.delivered_at and store.hook_misses(qid) == 1


def test_verification_is_scoped_to_the_workers_herdr_server(store, fake_herdr, monkeypatch, tpath):
    qid = answered_busy(store, monkeypatch)
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
    store.db.execute("UPDATE question_ext SET herdr_server = '/other.sock' WHERE id = ?", (qid,))
    assert store.hook_unverified(service.herdr.server_id()) == []


def test_rows_at_the_miss_cap_never_open_the_writable_store(store, fake_herdr, monkeypatch, tpath):
    """#26 L2: the fast path mirrors hook_candidates' filter."""
    from herdr_inbox import hook

    qid = answered_busy(store, monkeypatch)
    store.db.execute("INSERT INTO question_ext (id, group_id, hook_misses) VALUES (?, ?, 2)"
                     " ON CONFLICT(id) DO UPDATE SET hook_misses = 2", (qid, qid))
    assert not hook._has_candidates("aaaaaaaa-0000-4000-8000-00000000000a")
    store.db.execute("UPDATE question_ext SET hook_misses = 0, agent_gone_at = ? WHERE id = ?", (st.now(), qid))
    assert not hook._has_candidates("aaaaaaaa-0000-4000-8000-00000000000a")


def test_a_session_without_the_hook_keeps_the_busy_grace_typing(store, fake_herdr, monkeypatch):
    qid = answered_busy(store, monkeypatch)
    store.db.execute("UPDATE question_ext SET pending_since = ? WHERE id = ?",
                     (st.now(-st.timedelta(minutes=5)), qid))
    monkeypatch.setenv("FAKE_HERDR_SESSION", "aaaaaaaa-0000-4000-8000-00000000000a")
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


def test_a_session_without_answers_writes_only_the_minutely_liveness_stamp(store, fake_herdr, tpath):
    from .test_hook_launch import ROOT, run_hook as launch

    proc = launch("PostToolBatch", ROOT, tpath.parent / "data", payload=payload(tpath))
    assert proc.returncode == 0 and proc.stdout == ""
    row = store.db.execute("SELECT transcript_path, hook_delivery, hook_seen_at FROM sessions").fetchall()
    assert len(row) == 1 and row[0][0] is None and row[0][1] is None and row[0][2]
    launch("PostToolBatch", ROOT, tpath.parent / "data", payload=payload(tpath))
    assert store.db.execute("SELECT hook_seen_at FROM sessions").fetchone()[0] == row[0][2]  # throttled


def test_a_fresh_liveness_stamp_is_checked_read_only(store, fake_herdr, monkeypatch, tpath):
    """#26 final-pass NIT: the common path (stamp under a minute old) opens no writable
    connection."""
    import sqlite3

    from herdr_inbox import hook

    hook._touch("aaaaaaaa-0000-4000-8000-00000000000a")  # stale/missing: one write
    opened = []
    real = sqlite3.connect
    monkeypatch.setattr(sqlite3, "connect", lambda db, *a, **k: opened.append(str(db)) or real(db, *a, **k))
    hook._touch("aaaaaaaa-0000-4000-8000-00000000000a")
    assert opened and all("mode=ro" in d for d in opened)


def test_hook_delivery_column_is_no_longer_written(store, fake_herdr, monkeypatch, tpath):
    run_hook("session-start", {**payload(tpath), "source": "startup"})
    qid = answered_busy(store, monkeypatch)
    assert context(run_hook("post-tool-batch", payload(tpath)), "PostToolBatch")
    assert store.db.execute("SELECT hook_delivery FROM sessions WHERE session_id = 'aaaaaaaa-0000-4000-8000-00000000000a'").fetchone()[0] is None
    assert store.get(qid).delivered_at
