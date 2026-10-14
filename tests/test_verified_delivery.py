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

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")


@pytest.fixture(autouse=True)
def quick_verify(monkeypatch):
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.3)


@pytest.fixture
def live_session(fake_herdr):
    """The asking session's transcript already exists, as for any running Claude session."""
    path = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-fake-project" / "sess-A.jsonl"
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
    assert [a.id for a in store.unacknowledged_answers("sess-A", "w1:p1")] == [qid]  # check_answers has it
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
    assert prompts(fake_herdr) == []  # never typed into a busy pane
    assert q.delivery_error == "pending: waiting for the agent to be idle (it is working)"
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
    assert [a.id for a in store.unacknowledged_answers("sess-A", "w1:p1")] == [qid]


def test_hook_records_the_transcript_path_used_for_verification(store, fake_herdr, monkeypatch, tmp_path, capsys):
    path = tmp_path / "custom" / "sess-A.jsonl"
    path.parent.mkdir()
    path.write_text("")
    monkeypatch.setattr("sys.stdin", __import__("io").StringIO(json.dumps(
        {"session_id": "sess-A", "source": "startup", "transcript_path": str(path)})))
    assert hook.run("session-start") == 0
    assert store.session_transcript("sess-A") == str(path)
    assert transcript.find("sess-A", store.session_transcript("sess-A")) == path


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


async def test_receipt_mentions_check_answers(fake_herdr):
    from mcp import Client

    from herdr_inbox.server import mcp

    async with Client(mcp) as client:
        r = (await client.call_tool("ask_operator", {"question": "Q?"})).structured_content
    assert "call check_answers" in r["message"]


async def test_popup_shows_pending_delivery(store, fake_herdr, monkeypatch):
    qid = ask(store)
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "Other?")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.answer(store, qid, "yes")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        lv = app.query_one(GroupListView)
        body = lv.body_text()
        assert f"⏳ Q{qid} pending delivery" in body and "waiting for the agent to be idle" in body
        assert "1 pending delivery" in str(lv.query_one("#list-title").render())


def test_migration_makes_unverified_deliveries_unseen_once(tmp_path, fake_herdr, monkeypatch):
    """Rows like Q28/Q34/Q36-Q39: delivered + acknowledged by 0.4.0 without verification."""
    path = tmp_path / "old.db"
    store = Store(path)
    qid = ask(store)
    verified = ask(store, "Verified?")
    service.answer(store, verified, "ok")  # verified in this version
    store.set_answer(qid, "yes")
    store.db.execute("UPDATE questions SET delivered_at = answered_at, acknowledged_at = answered_at WHERE id = ?", (qid,))
    store.db.execute("DELETE FROM meta WHERE key = ?", (RESET_KEY,))  # as if this DB predates 0.5.1
    store.close()
    store = Store(path)
    assert store.get(qid).acknowledged_at is None and store.get(verified).acknowledged_at
    assert [a.id for a in store.unacknowledged_answers("sess-A", "w1:p1")] == [qid]
    store.acknowledge([qid])  # the agent fetched it; a later open must not reset it again
    store.close()
    assert Store(path).get(qid).acknowledged_at


def test_cli_redeliver(store, fake_herdr, monkeypatch, capsys):
    qid = ask(store)
    store.set_answer(qid, "yes")
    store.db.execute("UPDATE questions SET delivered_at = answered_at, acknowledged_at = answered_at WHERE id = ?", (qid,))
    assert cli.main(["redeliver", f"Q{qid}", "Q999"]) == 1
    out = capsys.readouterr()
    assert f"Q{qid}: delivered (verified)" in out.out and "Q999: not an answered question" in out.err
    assert store.get(qid).verified_at and prompts(fake_herdr)[-1].startswith(f"[inbox] Q{qid} answered")
