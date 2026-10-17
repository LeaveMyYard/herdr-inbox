"""Live 2026-10-17 ("a huge amount of questions I responded to long ago show as failed"):
16 answers in the popup's failed pile. Shapes from the operator's database and transcripts
(error strings are the inbox's own constants, as stored; texts neutralised).

- 6 x "not verified": herdr had resumed the pane's OLD session id; the old transcript still
  ended in `continued-in` -> NEW, so the inbox typed and watched NEW's transcript, while the
  typed batch landed as the first record of the resumed OLD one.
- Q156 "agent gone: conversation changed in pane w6:p57": typed the same way, landed in OLD.
- 9 x "agent gone: no live agent in pane w6:p48": filed under the dead pane id of a moved pane
  (the MCP server's HERDR_PANE_ID), for a conversation that no longer runs anywhere.
- acknowledged answers still carrying "agent gone"/"not verified" errors."""

import json

import pytest

from herdr_inbox import service, store as st, transcript, worker
from herdr_inbox.service import Caller

from .test_session_continuation import NEW, OLD, tdir, write

NOT_VERIFIED = "not verified: herdr accepted the text but it never reached the conversation"
GONE_P48 = "agent gone: no live agent in pane w6:p48"
CHANGED_P57 = "agent gone: conversation changed in pane w6:p57"


def agents(*pairs):
    return json.dumps([{"pane_id": p, "tab_id": "w6:t4T", "agent": "claude", "agent_status": "idle",
                        "agent_session": {"value": s}} for p, s in pairs])


@pytest.fixture(autouse=True)
def _fresh():
    getattr(service, "_reconciled_at", {}).clear()


def continued_then_resumed(batch_text=None):
    """OLD -> NEW continuation, then herdr resumed OLD: the first record after the continued-in
    is the typed batch (captured order: continued-in, cost-state, user ...)."""
    write(OLD, [{"type": "user", "message": {"role": "user", "content": "earlier"}},
                {"type": "continued-in", "timestamp": "2026-10-16T11:22:22.458Z", "sessionId": OLD,
                 "continuedInSessionId": NEW},
                {"type": "cost-state", "sessionId": OLD}])
    write(NEW, [{"type": "ai-title", "aiTitle": "x", "sessionId": NEW}])
    if batch_text:
        write(OLD, [{"type": "user", "timestamp": st.now(), "message": {"role": "user", "content": batch_text}}])


def filed(store, pane, session, text, answer):
    qid = service.ask(store, Caller(pane, "w6:t3K", "node", session, "/r"), text)["id"]
    store.set_draft(qid, service.make_detail(answer))
    service.submit(store, [qid], deliver=False)
    return store.get(qid)


def as_stored(store, qid, error, acknowledged=False):
    store.db.execute("UPDATE questions SET delivery_error = ?, delivery_attempts = 4, acknowledged_at = ?"
                     " WHERE id = ?", (error, st.now() if acknowledged else None, qid))
    if error.startswith("agent gone"):
        store.db.execute("INSERT INTO question_ext (id, group_id, agent_gone_at) VALUES (?, ?, ?)"
                         " ON CONFLICT(id) DO UPDATE SET agent_gone_at = excluded.agent_gone_at", (qid, qid, st.now()))


def test_typed_into_a_resumed_old_session_is_verified_at_once(store, fake_herdr, monkeypatch):
    """The cause of the 6 'not verified': watch the reported session's transcript too."""
    continued_then_resumed()
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)  # herdr reports the resumed OLD id
    monkeypatch.setenv("FAKE_HERDR_LAND_SESSION", OLD)  # and the typed text lands there
    q = filed(store, "w1:p1", NEW, "Three big prod analytics columns: views?", "yes, masked")
    service.deliver(store, q)
    q = store.get(q.id)
    assert q.delivery_error is None and q.acknowledged_at and q.verified_at


def test_reconcile_verifies_the_not_verified_answers_that_landed(store, fake_herdr):
    q1 = filed(store, "w6:p3R", NEW, "Three big prod analytics columns: views?", "yes")
    q2 = filed(store, "w6:p3R", NEW, "Where do the keys live?", "Secrets Manager")
    for q in (q1, q2):
        as_stored(store, q.id, NOT_VERIFIED)
    batch = service.herdr.clean(service.delivery_text([store.get(q1.id), store.get(q2.id)]), limit=10**9)
    continued_then_resumed(batch)
    store.record_session(OLD, str(tdir() / f"{OLD}.jsonl"))
    assert {q.id for q in store.failed_deliveries()} == {q1.id, q2.id}
    worker.tick(store)
    assert store.failed_deliveries() == []
    for q in (q1, q2):
        r = store.get(q.id)
        assert r.acknowledged_at and r.verified_at and r.delivery_error is None


def test_reconcile_finds_a_changed_conversation_answer_in_the_panes_current_session(store, fake_herdr, monkeypatch):
    """Q156: 'conversation changed in pane w6:p57', but it landed in what p57 runs now."""
    q = filed(store, "w6:p57", NEW, "PR ghost#1927: screenshots blocked, merge anyway?", "merge")
    as_stored(store, q.id, CHANGED_P57)
    write(OLD, [{"type": "user", "timestamp": st.now(),
                 "message": {"role": "user", "content": service.herdr.clean(service.delivery_text([store.get(q.id)]),
                                                                          limit=10**9)}}])
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w6:p57", OLD)))
    worker.tick(store)
    r = store.get(q.id)
    assert r.verified_at and r.acknowledged_at and not store.agent_gone(q.id)


def test_an_acknowledged_answer_never_keeps_a_failure(store, fake_herdr):
    q = filed(store, "w6:p48", NEW, "Old pane question?", "done")
    as_stored(store, q.id, GONE_P48, acknowledged=True)
    worker.tick(store)
    assert store.get(q.id).delivery_error is None
    q2 = filed(store, "w6:p48", NEW, "Fetched by check_answers?", "ok")
    as_stored(store, q2.id, GONE_P48)
    store.acknowledge([q2.id])  # check_answers
    assert store.get(q2.id).delivery_error is None


def test_a_truly_gone_answer_that_never_landed_stays_visible(store, fake_herdr, monkeypatch):
    q = filed(store, "w6:p48", NEW, "table-page: wording for the error state?", "Could not load")
    as_stored(store, q.id, GONE_P48)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w6:p57", OLD)))
    worker.tick(store)
    assert [r.id for r in store.failed_deliveries()] == [q.id]


def test_a_question_from_a_moved_pane_is_filed_under_the_pane_it_runs_in(store, fake_herdr, monkeypatch):
    """Q158-Q166: the MCP server's HERDR_PANE_ID was the pane it started in (w6:p48)."""
    monkeypatch.setenv("HERDR_PANE_ID", "w6:p48")
    monkeypatch.setenv("FAKE_HERDR_GONE_PANES", "w6:p48")
    monkeypatch.setenv("CLAUDE_CODE_SESSION_ID", NEW)
    monkeypatch.setenv("FAKE_HERDR_SESSION", NEW)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", agents(("w6:p57", NEW), ("w6:p32", "other")))
    caller = service.resolve_caller()
    assert caller.pane_id == "w6:p57" and caller.session_id == NEW


def test_reconcile_backs_off_per_answer(store, fake_herdr, monkeypatch):
    q = filed(store, "w6:p48", NEW, "Backoff?", "x")
    as_stored(store, q.id, GONE_P48)
    fake_herdr.reset()
    service.reconcile_failed(store)
    service.reconcile_failed(store)
    assert len(fake_herdr.calls_of("agent", "list")) == 1


# -- operator Q167 ("Both"): one collapsed row with d-hides-all, and auto-archive after 24 h --

from herdr_inbox.tui import GroupListView, InboxApp  # noqa: E402


_made = iter(range(10**6))


def gone_rows(store, n):
    ids = []
    for _ in range(n):
        q = filed(store, "w6:p48", NEW, f"Gone question {next(_made)}?", "a")
        as_stored(store, q.id, GONE_P48)
        ids.append(q.id)
    return ids


async def test_q167_gone_answers_collapse_into_one_row_and_d_hides_them_all(store, fake_herdr):
    ids = gone_rows(store, 3)
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "A live question?")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert "⚠ 3 answers for agents that are gone · d hides them all" in body
        assert body.count("agent closed") == 0  # no per-answer rows
        await pilot.press("down", "d")
        await pilot.pause()
        assert "✕ answers for gone agents · hidden" in app.query_one(GroupListView).body_text()
    assert store.failed_deliveries() == []
    assert all(store.get(i).acknowledged_at is None for i in ids)  # check_answers still has them


async def test_q167_a_single_gone_answer_keeps_its_own_row(store, fake_herdr):
    gone_rows(store, 1)
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "A live question?")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert "agent closed" in body and "agents that are gone" not in body


async def test_q167_a_gone_answer_arriving_later_never_joins_the_open_summary(store, fake_herdr):
    gone_rows(store, 2)
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "A live question?")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        (late,) = gone_rows(store, 1)
        app.poll()
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert "⚠ 2 answers for agents that are gone" in body and f"Q{late} agent closed" in body
        await pilot.press("down", "d")
        await pilot.pause()
    assert [q.id for q in store.failed_deliveries()] == [late]  # only what the row showed was hidden


def test_q167_gone_answers_leave_the_popup_after_a_day(store, fake_herdr):
    old, new = gone_rows(store, 2)
    store.db.execute("UPDATE question_ext SET agent_gone_at = ? WHERE id = ?", (st.now(-st.timedelta(hours=25)), old))
    worker.tick(store)
    assert [q.id for q in store.failed_deliveries()] == [new]
    assert store.get(old).acknowledged_at is None  # still for check_answers


# -- the agent answered its own questions with the CLI (live 2026-10-17, Q159-Q166) ----------
# Captured shapes from the resumed old session's transcript: the assistant's Bash tool_use
# running `$B answer 159 "<answer>"`, then the tool_result with our CLI's own output line.

def own_cli_answer_records(qid, answer, at):
    return [
        {"type": "assistant", "timestamp": at, "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_x", "name": "Bash", "input": {
                "command": f"B=~/.claude/plugins/data/herdr-inbox-herdr-inbox/venv-0.10.4/bin/inbox\n"
                           f"$B answer {qid} \"{answer}\""}}]}},
        {"type": "user", "timestamp": at, "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_x",
             "content": f"Q{qid} answered, not delivered: no live agent in pane w6:p48\n"}]}},
    ]


def test_reconcile_counts_an_answer_the_agent_gave_itself_as_seen(store, fake_herdr):
    q = filed(store, "w6:p48", NEW, "P7 domains: v1 whitelisted/blacklisted pages are placeholders; port?",
              "Keep as built (already merged in #1943): faithful empty placeholder")
    as_stored(store, q.id, GONE_P48)
    # OLD -> NEW, then the operator resumed OLD; the agent there answered with the CLI
    write(OLD, [{"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW},
                {"type": "user", "timestamp": st.now(), "message": {"role": "user", "content": "back"}}]
          + own_cli_answer_records(q.id, q.answer, st.now()))
    store.record_session(OLD, str(tdir() / f"{OLD}.jsonl"))
    write(NEW, [{"type": "ai-title", "aiTitle": "x"}])
    worker.tick(store)
    r = store.get(q.id)
    assert r.acknowledged_at and r.delivery_error is None and not store.agent_gone(q.id)
    assert store.failed_deliveries() == []


def test_a_mere_id_mention_with_a_short_answer_is_not_evidence(store, fake_herdr):
    q = filed(store, "w6:p48", NEW, "Short one?", "yes")
    as_stored(store, q.id, GONE_P48)
    write(NEW, [{"type": "user", "timestamp": st.now(), "message": {"role": "user",
                 "content": f"about Q{q.id}: yes, later"}}])
    worker.tick(store)
    assert [r.id for r in store.failed_deliveries()] == [q.id]


def test_the_cli_answer_from_the_asking_conversation_is_seen_at_once(store, fake_herdr, monkeypatch):
    """`inbox answer` run by the asking agent itself (even from its resumed older session)."""
    from herdr_inbox.cli import main

    write(OLD, [{"type": "continued-in", "sessionId": OLD, "continuedInSessionId": NEW},
                {"type": "user", "message": {"role": "user", "content": "back"}}])
    store.record_session(OLD, str(tdir() / f"{OLD}.jsonl"))
    write(NEW, [{"type": "ai-title", "aiTitle": "x"}])
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", NEW, "/r"), "Relay the operator's call?")["id"]
    monkeypatch.setenv("CLAUDE_CODE_SESSION_ID", OLD)
    monkeypatch.setenv("FAKE_HERDR_SESSION", OLD)
    fake_herdr.reset()
    assert main(["answer", str(qid), "Keep as built, decided in chat"]) == 0
    q = store.get(qid)
    assert q.status == "answered" and q.acknowledged_at and q.delivery_error is None
    assert not fake_herdr.calls_of("agent", "prompt")


def test_the_cli_answer_from_another_agent_still_delivers(store, fake_herdr, monkeypatch):
    from herdr_inbox.cli import main

    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", NEW, "/r"), "Someone else answers?")["id"]
    monkeypatch.setenv("FAKE_HERDR_SESSION", NEW)
    monkeypatch.setenv("CLAUDE_CODE_SESSION_ID", "someone-else")  # another agent's process
    assert main(["answer", str(qid), "go"]) == 0
    assert fake_herdr.calls_of("agent", "prompt")


def test_a_retry_never_types_what_already_landed_in_a_resumed_old_session(store, fake_herdr, monkeypatch):
    """The no-double-delivery guard: the pre-type check reads every related transcript."""
    q = filed(store, "w1:p1", NEW, "Landed in the resumed old one?", "yes it did")
    as_stored(store, q.id, NOT_VERIFIED)
    continued_then_resumed(service.herdr.clean(service.delivery_text([store.get(q.id)]), limit=10**9))
    store.record_session(OLD, str(tdir() / f"{OLD}.jsonl"))
    monkeypatch.setenv("FAKE_HERDR_SESSION", NEW)
    fake_herdr.reset()
    service.deliver(store, store.get(q.id), retry=True, force=True)  # even Send now
    assert not fake_herdr.calls_of("agent", "prompt")
    assert store.get(q.id).acknowledged_at
