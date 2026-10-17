"""0.5.1: a typed answer counts as delivered (and seen) only once it shows up in the asking
session's transcript; herdr accepting the prompt is not enough (live 0.4.0 bug)."""

import json
import os
from pathlib import Path

import pytest

from herdr_inbox import cli, hook, service, transcript, tui
from herdr_inbox.service import Caller
from herdr_inbox.store import RESET_KEY, Store
from herdr_inbox.tui import GroupListView, InboxApp

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")


@pytest.fixture(autouse=True)
def quick_verify(monkeypatch):
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.3)


@pytest.fixture
def live_session(fake_herdr):
    """The asking session's transcript already exists, as for any running Claude session."""
    path = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-fake-project" / "aaaaaaaa-0000-4000-8000-00000000000a.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"type": "user", "message": {"content": "hello"}}) + "\n")
    return path


def ask(store, text="Merge it?"):
    return service.ask(store, A, text, options=["yes", "no"])["id"]


def prompts(fake):
    return [c[3] for c in fake.calls_of("agent", "prompt")]


def test_prompt_that_never_lands_is_not_delivered_and_not_seen(store, fake_herdr, monkeypatch, live_session):
    """The live bug: herdr exit 0, nothing in the conversation."""
    qid = ask(store)
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    q = service.answer(store, qid, "yes")
    assert prompts(fake_herdr)  # herdr accepted it
    assert q.delivered_at is None and q.acknowledged_at is None and q.verified_at is None
    assert q.delivery_error == service.NOT_VERIFIED
    assert [a.id for a in store.unacknowledged_answers("aaaaaaaa-0000-4000-8000-00000000000a", "w1:p1")] == [qid]  # check_answers has it
    assert [f.id for f in store.failed_deliveries()] == [qid]  # the popup shows it


def test_not_verified_is_retried_and_then_verified(store, fake_herdr, monkeypatch, live_session):
    qid = ask(store)
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    service.answer(store, qid, "yes")
    monkeypatch.setenv("FAKE_HERDR_LAND", "user")
    service.retry_deliveries(store, "w1:p1")
    q = store.get(qid)
    assert q.delivered_at and q.verified_at and q.acknowledged_at and q.delivery_error is None


def test_queued_mid_turn_record_counts_as_landed(store, fake_herdr, monkeypatch):
    qid = ask(store)
    monkeypatch.setenv("FAKE_HERDR_LAND", "queue")  # Claude Code's queue-operation enqueue
    assert service.answer(store, qid, "yes").verified_at


def test_busy_agent_keeps_the_answer_pending(store, fake_herdr, monkeypatch):
    qid = ask(store)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    q = service.answer(store, qid, "yes")
    assert prompts(fake_herdr) == []  # not typed at once into a busy pane (0.5.4: BUSY_GRACE)
    assert q.delivery_error == "pending: busy: the agent is working"
    assert q.acknowledged_at is None and q.delivery_attempts == 0
    monkeypatch.setenv("FAKE_HERDR_STATUS", "blocked")
    service.retry_deliveries(store, "w1:p1")
    assert prompts(fake_herdr) == []
    monkeypatch.setenv("FAKE_HERDR_STATUS", "done")  # at rest again: the retry loop delivers
    service.retry_deliveries(store, "w1:p1")
    assert store.get(qid).verified_at and len(prompts(fake_herdr)) == 1


def test_unknown_transcript_means_sent_but_unseen(store, fake_herdr, monkeypatch, tmp_path):
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path / "elsewhere"))
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    qid = ask(store)
    q = service.answer(store, qid, "yes")
    assert q.delivered_at and q.acknowledged_at is None and q.verified_at is None
    assert [a.id for a in store.unacknowledged_answers("aaaaaaaa-0000-4000-8000-00000000000a", "w1:p1")] == [qid]


def test_hook_records_the_transcript_path_used_for_verification(store, fake_herdr, monkeypatch, tmp_path, capsys):
    path = tmp_path / "custom" / "aaaaaaaa-0000-4000-8000-00000000000a.jsonl"
    path.parent.mkdir()
    path.write_text("")
    monkeypatch.setattr("sys.stdin", __import__("io").StringIO(json.dumps(
        {"session_id": "aaaaaaaa-0000-4000-8000-00000000000a", "source": "startup", "transcript_path": str(path)})))
    assert hook.run("session-start") == 0
    assert store.session_transcript("aaaaaaaa-0000-4000-8000-00000000000a") == str(path)
    assert transcript.find("aaaaaaaa-0000-4000-8000-00000000000a", store.session_transcript("aaaaaaaa-0000-4000-8000-00000000000a")) == path


def test_transcript_parsing_is_lenient_and_exact(tmp_path):
    path = tmp_path / "t.jsonl"
    path.write_text("\n".join([
        "not json",
        json.dumps({"type": "user", "message": {"content": [{"type": "tool_result", "content": "[inbox] Q7"}]}}),
        json.dumps({"type": "assistant", "message": {"content": "[inbox] Q7 answered"}}),
        json.dumps({"type": "user", "message": {"content": "[inbox] Q70 answered — x: y"}}),
    ]) + "\n")
    assert not transcript.landed_since(path, 0, [7])  # tool results, assistant text, Q70 don't count
    with open(path, "a") as fh:
        fh.write(json.dumps({"type": "attachment", "attachment": {"type": "queued_command",
                                                                   "prompt": "[inbox] answered — Q7 a: b · Q8 c: d"}}) + "\n")
    assert transcript.landed_since(path, 0, [7, 8])
    assert not transcript.landed_since(path, path.stat().st_size, [7, 8])  # only after the offset


def test_receipt_and_hook_mention_check_answers():
    from herdr_inbox.hook import SESSION_RULES

    assert "If one seems overdue, call check_answers" in SESSION_RULES and len(SESSION_RULES.split()) < 250


async def test_receipt_mentions_check_answers(migrated_db, fake_herdr):
    from mcp import Client

    from herdr_inbox.server import mcp

    async with Client(mcp) as client:
        r = (await client.call_tool("ask_operator", {"question": "Q?"})).structured_content
    assert "call check_answers" in r["message"]


async def test_popup_shows_pending_delivery(store, fake_herdr, monkeypatch):
    qid = ask(store)
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Other?")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.answer(store, qid, "yes")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        lv = app.query_one(GroupListView)
        body = lv.body_text()
        assert f"⏳ Q{qid} answered · waiting to deliver: agent busy — will send in" in body  # 0.5.4: says why and when
        assert "1 pending delivery" in str(lv.query_one("#list-title").render())


def _old_server_delivery(store, qid, when="2026-10-14T12:53:08+00:00"):
    """What a pre-0.5.1 server records on "send": delivered and seen in the same instant."""
    store.db.execute("UPDATE questions SET delivered_at = ?, acknowledged_at = ?, delivery_error = NULL"
                     " WHERE id = ?", (when, when, qid))


def test_migration_uses_the_verifier(tmp_path, fake_herdr, live_session):
    """M1 (review of #9): only answers missing from the transcript are un-seen; the ones that
    did land (like Q22/Q26 in the live transcript) are marked verified instead."""
    path = tmp_path / "old.db"
    store = Store(path, migrate=True)
    landed, lost = ask(store, "Landed?"), ask(store, "Lost?")
    for qid in (landed, lost):
        store.set_answer(qid, "yes")
        _old_server_delivery(store, qid, "2026-10-01T00:00:00+00:00")
    with open(live_session, "a") as fh:
        fh.write(json.dumps({"type": "user", "message": {"content": f"[inbox] Q{landed} answered — Landed?: yes"}}) + "\n")
    store.db.execute("DELETE FROM meta WHERE key = ?", (RESET_KEY,))  # as if this DB predates 0.5.1
    store.close()
    store = Store(path)
    assert store.get(landed).acknowledged_at and store.get(landed).verified_at
    assert store.get(lost).acknowledged_at is None
    assert [a.id for a in store.unacknowledged_answers("aaaaaaaa-0000-4000-8000-00000000000a", "w1:p1")] == [lost]


def test_old_server_deliveries_are_rechecked_on_every_open_and_cannot_loop(store, fake_herdr, live_session):
    """M2 (review of #9): a 0.4/0.5.0 server still running types a pending answer into a busy
    pane and marks it seen; the next open of the DB notices and un-sees it, at most twice."""
    from datetime import timedelta

    from herdr_inbox.store import now

    qid = ask(store)
    store.set_answer(qid, "yes")
    for round_ in range(1, 4):
        _old_server_delivery(store, qid, now(-timedelta(seconds=60)))  # settled (0.6.1: REQUEUE_SETTLE)
        reopened = Store(store.path)
        expected_seen = round_ > 2  # MAX_RESETS: the third re-ack by an old server stays
        assert (reopened.get(qid).acknowledged_at is not None) == expected_seen
        reopened.close()
    assert len(fake_herdr.calls_of("agent", "prompt")) == 0  # never re-typed by the reset itself


def test_check_answers_acknowledgement_is_not_the_old_server_signature(store, fake_herdr, live_session):
    from herdr_inbox.store import now

    qid = ask(store)
    store.set_answer(qid, "yes")
    store.db.execute("UPDATE questions SET delivered_at = '2026-10-14T00:00:00+00:00' WHERE id = ?", (qid,))
    store.acknowledge([qid])  # the agent fetched it later
    assert Store(store.path).get(qid).acknowledged_at


def test_never_landing_prompt_is_typed_at_most_twice(store, fake_herdr, monkeypatch, live_session):
    """H1 (review of #9): a verification miss must not become the same answer every tick."""
    qid = ask(store)
    monkeypatch.setenv("FAKE_HERDR_LAND", "none")
    service.answer(store, qid, "yes")
    for _ in range(8):
        service.retry_deliveries(store, "w1:p1")
    assert len(prompts(fake_herdr)) == service.MAX_UNVERIFIED_ATTEMPTS == 2
    q = store.get(qid)
    assert q.acknowledged_at is None and q.delivery_error == service.NOT_VERIFIED
    assert [f.id for f in store.failed_deliveries()] == [qid]  # ⚠ in the popup


def test_list_content_and_collapsed_paste_records_count_as_landed(tmp_path):
    path = tmp_path / "t.jsonl"
    path.write_text("\n".join([
        json.dumps({"type": "user", "message": {"content": [{"type": "text", "text": "[inbox] Q5 answered — ok?: yes"}]}}),
        # captured from a real Claude Code session: a long prompt is stored wrapped
        json.dumps({"type": "user", "message": {"content":
                    '\n\n<pasted_content id="ec16">\n[inbox] answered — Q9001 probe: x · Q9002 end: y</pasted_content>'}}),
    ]) + "\n")
    assert transcript.landed_since(path, 0, [5])
    assert transcript.landed_since(path, 0, [9001, 9002])
    assert transcript.landed_ids(path, [5, 9001, 7]) == {5, 9001}


def test_cli_redeliver(store, fake_herdr, monkeypatch, capsys):
    qid = ask(store)
    store.set_answer(qid, "yes")
    store.db.execute("UPDATE questions SET delivered_at = answered_at, acknowledged_at = answered_at WHERE id = ?", (qid,))
    assert cli.main(["redeliver", f"Q{qid}", "Q999"]) == 1
    out = capsys.readouterr()
    assert f"Q{qid}: delivered (verified)" in out.out and "Q999: not an answered question" in out.err
    assert store.get(qid).verified_at and prompts(fake_herdr)[-1].startswith(f"[inbox] Q{qid} answered")


def test_a_mention_of_a_q_id_is_not_a_delivery_of_it(tmp_path):
    """Final review of #9, M1: only the positions our own format writes ids in count."""
    path = tmp_path / "t.jsonl"
    path.write_text("\n".join(json.dumps({"type": "user", "message": {"content": c}}) for c in [
        "[inbox] Q27 answered — which region?: same as Q28",
        "[inbox] answered — Q30 Re Q31: ok · Q32 next?: like Q33 · Q34",
    ]) + "\n")
    assert transcript.landed_ids(path, [27, 28, 30, 31, 32, 33, 34]) == {27, 30, 32}
    assert not transcript.landed_since(path, 0, [28])
    assert transcript.landed_since(path, 0, [30, 32])


def test_seven_day_sweep_continues_past_one_batch(tmp_path, fake_herdr, live_session, monkeypatch):
    """Final review of #9, Low: the done-flag is set only by a pass that saw everything."""
    from herdr_inbox import store as store_mod

    monkeypatch.setattr(store_mod, "RECONCILE_BATCH", 2)
    path = tmp_path / "old.db"
    store = Store(path, migrate=True)
    ids = [ask(store, f"Q{i}?") for i in range(5)]
    for qid in ids:
        store.set_answer(qid, "yes")
        _old_server_delivery(store, qid, "2026-10-01T00:00:00+00:00")
    store.db.execute("DELETE FROM meta WHERE key = ?", (RESET_KEY,))
    store.close()
    for _ in range(3):
        Store(path).close()
    store = Store(path)
    assert all(store.get(qid).acknowledged_at is None for qid in ids)
    assert store.db.execute("SELECT 1 FROM meta WHERE key = ?", (RESET_KEY,)).fetchone()


def test_verified_delivery_is_written_in_one_transaction(store, fake_herdr, live_session):
    """Final review of #9, Low: no other process may see delivered+acked without verified_at."""
    qid = ask(store)
    store.set_answer(qid, "yes")
    seen = []
    store.db.set_trace_callback(seen.append)
    store.record_delivery(qid, None, verified=True)
    store.db.set_trace_callback(None)
    writes = [s for s in seen if s.lstrip().upper().startswith(("UPDATE", "INSERT", "BEGIN", "COMMIT"))]
    assert writes[0].startswith("BEGIN") and writes[-1] == "COMMIT"
