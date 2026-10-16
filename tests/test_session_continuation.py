"""Claude Code session continuation (live 2026-10-16, pane "Ghostpaw frontend v2 rework"):
the conversation moved from session OLD to NEW in the same process. The old transcript ends
with a `continued-in` record; nothing in the new one points back; herdr (and the process
env) keep reporting OLD. The inbox must follow the record: questions, verification, hook
and rewake delivery all belong to NEW."""

import json
import os
from pathlib import Path

import pytest

from herdr_inbox import herdr, rewake, service, store as st, transcript, worker
from herdr_inbox.service import Caller

OLD = "a58e3f68-0000-4000-8000-000000000001"
NEW = "06acc80f-0000-4000-8000-000000000002"


def tdir():
    d = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-fake-project"
    d.mkdir(parents=True, exist_ok=True)
    return d


def write(session, records):
    path = tdir() / f"{session}.jsonl"
    with open(path, "a") as fh:
        for r in records:
            fh.write(json.dumps(r) + "\n")
    return path


def continued(old=OLD, new=NEW, after=("last-prompt", "cost-state")):
    """The captured shape (2.1.301): continued-in, then only bookkeeping records."""
    write(old, [{"type": "user", "message": {"role": "user", "content": "earlier work"}, "sessionId": old},
                {"type": "continued-in", "timestamp": "2026-10-16T18:41:47.545Z", "sessionId": old,
                 "continuedInSessionId": new}]
          + [{"type": t, "sessionId": old} for t in after])
    write(new, [{"type": "ai-title", "aiTitle": "x", "sessionId": new}])


@pytest.fixture
def herdr_says_old(fake_herdr, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)  # herdr keeps the old id
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", NEW)  # typed text lands in the live conversation
    monkeypatch.setenv("CLAUDE_CODE_SESSION_ID", OLD)  # the MCP server's env, from before the switch


def test_follow_reads_the_captured_record(fake_herdr):
    continued()
    assert transcript.follow(OLD) == NEW
    assert transcript.follow(NEW) == NEW
    assert transcript.follow("unknown-session") == "unknown-session"


def test_follow_chains_and_ignores_a_resumed_old_session(fake_herdr):
    third = "11111111-0000-4000-8000-000000000003"
    continued()
    continued(NEW, third, after=())
    assert transcript.follow(OLD) == third
    write(third, [{"type": "continued-in", "sessionId": third, "continuedInSessionId": OLD}])
    assert transcript.follow(OLD) in (OLD, NEW, third)  # a cycle never loops
    other = "22222222-0000-4000-8000-000000000004"
    write(other, [{"type": "continued-in", "sessionId": other, "continuedInSessionId": NEW},
                  {"type": "user", "message": {"role": "user", "content": "resumed and used again"}}])
    assert transcript.follow(other) == other  # conversation went on in the old file: not followed


def test_live_agent_reports_the_live_conversation(herdr_says_old):
    continued()
    agent = herdr.live_agent("w1:p1")
    assert agent["session_id"] == NEW and agent["reported_session_id"] == OLD


def test_a_new_question_is_filed_under_the_live_conversation(store, herdr_says_old):
    continued()
    qid = service.ask(store, service.resolve_caller(), "After the switch?")["id"]
    assert store.get(qid).session_id == NEW


def test_an_answer_to_a_question_asked_before_the_switch_is_typed_and_verified(store, herdr_says_old, monkeypatch, fake_herdr):
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", OLD)
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", OLD, "/r"), "Before the switch?")["id"]
    continued()
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", NEW)
    service.answer(store, qid, "yes")
    q = store.get(qid)
    assert q.delivery_error is None and q.acknowledged_at  # typed, found in NEW's transcript
    assert q.session_id == OLD  # identity kept (qkey); resolved through the continuation
    assert len(fake_herdr.calls_of("agent", "prompt")) == 1


def test_a_question_filed_under_new_is_not_refused_as_changed(store, herdr_says_old):
    continued()
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", NEW, "/r"), "Filed under new?")["id"]
    service.answer(store, qid, "ok")
    assert store.get(qid).acknowledged_at


def test_the_worker_records_the_continuation_and_new_owns_old_rows(store, fake_herdr):
    from herdr_inbox import alerts

    q_open = service.ask(store, Caller("w9:p9", None, None, OLD, "/r"), "Open one?")["id"]
    a = alerts.raise_alert(store, Caller("w9:p9", None, None, OLD, "/r"), "Check it")["id"]
    continued()
    worker.tick(store)
    assert store.db.execute("SELECT continued_in FROM sessions WHERE session_id = ?", (OLD,)).fetchone()[0] == NEW
    assert store.get(q_open).session_id == OLD and alerts.get(store, a).session_id == OLD  # never rewritten
    new = Caller("w9:p9", None, None, NEW, "/r")
    assert [q.id for q in store.mine(NEW, None)] == [q_open]  # list_my_questions of NEW
    assert [x.id for x in alerts.mine(store, new)] == [a] and alerts.withdraw(store, new, a)["status"] == "withdrawn"
    # a row filed under OLD later (a stale process) is NEW's too, no second pass needed
    late = service.ask(store, Caller("w9:p9", None, None, OLD, "/r"), "Filed late under old?")["id"]
    assert late in [q.id for q in store.mine(NEW, None)]

def test_the_old_sessions_watcher_ends_and_the_new_one_wakes(store, herdr_says_old, monkeypatch):
    from herdr_inbox import proc

    monkeypatch.setattr(proc, "owning_claude", lambda: None)
    continued()

    class Clock:
        t = 0.0

        def __call__(self):
            return self.t

        def sleep(self, s):
            self.t += s

    c = Clock()
    assert rewake.run({"session_id": OLD}, clock=c, sleep=c.sleep) is None  # ended at the first pane check
    assert c.t <= rewake.PANE_CHECK_EVERY + 2 * rewake.WATCH_POLL
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", NEW, "/r"), "For the new one?")["id"]
    store.set_draft(qid, service.make_detail("yes"))
    service.submit(store, [qid], deliver=False)
    c = Clock()
    assert rewake.run({"session_id": NEW, "transcript_path": str(tdir() / f"{NEW}.jsonl")}, clock=c, sleep=c.sleep)


@pytest.mark.parametrize("bad", ["../../etc/passwd", "*", "a/b", "06ACC80F-0000-4000-8000-000000000002 ", "x" * 36, ""])
def test_a_continuation_id_that_is_not_a_uuid_is_never_followed(fake_herdr, bad):
    """Path traversal guard: the id becomes part of a transcript path."""
    write(OLD, [{"type": "continued-in", "sessionId": OLD, "continuedInSessionId": bad}])
    assert transcript.follow(OLD) == OLD


def test_a_remote_answer_in_flight_survives_the_continuation(store, fake_herdr, tmp_path, monkeypatch):
    """#41 High 1: qkey hashes the filed session; it never changes, so a queued answer applies."""
    from herdr_inbox import remote, remote_api
    from herdr_inbox.store import Store

    remote_db = Store(tmp_path / "remote.db")
    qid = service.ask(remote_db, Caller("w1:p1", "w1:t1", "tab", OLD, "/r"), "Remote, before the switch?")["id"]
    key = remote_api.qkey(remote_db, remote_db.get(qid))
    continued()
    worker.tick(remote_db)  # the remote's worker records the continuation
    assert remote_api.qkey(remote_db, remote_db.get(qid)) == key
    out = remote_api.apply(remote_db, {"request_id": "r1", "db_id": remote_api.db_id(remote_db), "id": qid,
                                       "qkey": key, "action": "answer", "detail": {"text": "applied"}})
    assert out["outcome"] == "applied" and remote_db.get(qid).answer == "applied"
    remote_db.close()


def test_a_hook_answer_that_landed_in_the_old_transcript_is_not_sent_again(store, herdr_says_old, monkeypatch):
    """#41 High 2 (the reviewer's repro): sent as hook context in OLD, landed in OLD.jsonl,
    then the continuation; verification must find it there."""
    from herdr_inbox import hook

    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", OLD, "/r"), "Busy answer?")["id"]
    service.answer(store, qid, "sure")
    old_path = write(OLD, [])
    text = hook.deliver({"session_id": OLD, "transcript_path": str(old_path)})
    assert text
    write(OLD, [{"type": "attachment", "timestamp": st.now(),
                 "attachment": {"type": "hook_additional_context", "content": [text], "hookEvent": "PostToolBatch"}}])
    continued()
    monkeypatch.setattr(service, "HOOK_VERIFY_SECONDS", 0.0)
    worker.tick(store)
    q = store.get(qid)
    assert q.delivered_at and q.acknowledged_at  # verified in OLD's transcript, not re-pended


@pytest.mark.parametrize("bad", ["*", "?", "[ab]", "..\\evil", "../evil", "a.b", "*-0000-4000-8000-000000000002"])
def test_find_never_globs_or_walks_with_a_bad_session_id(fake_herdr, bad):
    """#41 Medium: ids reach find from herdr, env and transcripts; never a glob or a path."""
    write(NEW, [])
    assert transcript.find(bad) is None
    write(OLD, [{"type": "continued-in", "sessionId": OLD, "continuedInSessionId": bad}])
    assert transcript.follow(OLD) == OLD


def test_follow_reads_only_what_a_live_transcript_appended(fake_herdr, monkeypatch):
    """#41 Low: a growing transcript is read incrementally, not its tail every lookup."""
    path = write(NEW, [{"type": "user", "message": {"role": "user", "content": "x" * 300_000}}])
    assert transcript.follow(NEW) == NEW
    reads = []
    real_open = open

    def counting_open(p, mode="r", *a, **k):
        fh = real_open(p, mode, *a, **k)
        if str(p) == str(path) and "b" in mode:
            real_read = fh.read
            def read(n=-1):
                data = real_read(n)
                reads.append(len(data))
                return data
            fh.read = read
        return fh
    monkeypatch.setattr("builtins.open", counting_open)
    write(NEW, [{"type": "user", "message": {"role": "user", "content": "y"}}])
    assert transcript.follow(NEW) == NEW
    assert reads and max(reads) < 200  # only the appended line



def resumed_old(store):
    """OLD continued in NEW (recorded), then the operator resumed OLD and used it again."""
    service.ask(store, Caller("w9:p9", None, None, OLD, "/r"), "In play before the switch?")
    continued()
    worker.tick(store)  # records OLD -> NEW
    write(OLD, [{"type": "user", "message": {"role": "user", "content": "back in the old one"}}])


def test_final_a_resumed_old_sessions_answer_never_goes_to_new_by_hook(store, fake_herdr, monkeypatch):
    """#41 final pass H1 (reproduced there): after a resume, NEW's hooks must not take OLD's answers."""
    from herdr_inbox import hook

    resumed_old(store)
    qid = service.ask(store, Caller("w9:p9", None, None, OLD, "/r"), "Asked in the resumed old session?")["id"]
    store.set_draft(qid, service.make_detail("for OLD"))
    service.submit(store, [qid], deliver=False)
    assert store.family(NEW) == [NEW]
    assert store.hook_candidates(NEW, service.MAX_HOOK_MISSES) == []
    assert not hook._has_candidates(NEW)
    assert hook.deliver({"session_id": NEW}) is None
    assert store.get(qid).delivered_at is None


def test_final_a_resumed_old_sessions_answer_never_goes_to_new_by_check_answers(store, fake_herdr):
    from herdr_inbox import alerts

    resumed_old(store)
    qid = service.ask(store, Caller("w9:p9", None, None, OLD, "/r"), "Asked in the resumed old session?")["id"]
    service.answer(store, qid, "for OLD", deliver=False)
    a = alerts.raise_alert(store, Caller("w9:p9", None, None, OLD, "/r"), "OLD's alert")["id"]
    assert store.unacknowledged_answers(NEW, None) == [] and store.mine(NEW, None) == []
    assert alerts.mine(store, Caller("w9:p9", None, None, NEW, "/r")) == []
    assert [q.id for q in store.unacknowledged_answers(OLD, None)] == [qid]
    assert [x.id for x in alerts.mine(store, Caller("w9:p9", None, None, OLD, "/r"))] == [a]


def test_final_the_family_still_holds_while_the_continuation_stands(store, fake_herdr):
    service.ask(store, Caller("w9:p9", None, None, OLD, "/r"), "In play?")
    continued()
    worker.tick(store)
    assert set(store.family(NEW)) == {NEW, OLD}


def test_final_the_lookup_caches_are_bounded(fake_herdr, monkeypatch):
    monkeypatch.setattr(transcript, "CACHE_MAX", 5)
    for i in range(20):
        sid = f"{i:08d}-0000-4000-8000-000000000000"
        write(sid, [{"type": "user", "message": {"role": "user", "content": "x"}}])
        transcript.follow(sid)
    assert len(transcript._found) <= 5 and len(transcript._follow_cache) <= 5
