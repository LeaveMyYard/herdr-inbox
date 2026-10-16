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
    assert q.session_id == NEW  # re-homed
    assert len(fake_herdr.calls_of("agent", "prompt")) == 1


def test_a_question_filed_under_new_is_not_refused_as_changed(store, herdr_says_old):
    continued()
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", NEW, "/r"), "Filed under new?")["id"]
    service.answer(store, qid, "ok")
    assert store.get(qid).acknowledged_at


def test_the_worker_re_homes_old_rows_and_alerts(store, fake_herdr):
    from herdr_inbox import alerts

    q_open = service.ask(store, Caller("w9:p9", None, None, OLD, "/r"), "Open one?")["id"]
    a = alerts.raise_alert(store, Caller("w9:p9", None, None, OLD, "/r"), "Check it")["id"]
    continued()
    worker.tick(store)
    assert store.get(q_open).session_id == NEW and alerts.get(store, a).session_id == NEW
    assert store.db.execute("SELECT continued_in FROM sessions WHERE session_id = ?", (OLD,)).fetchone()[0] == NEW


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
