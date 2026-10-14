"""Grouped questions, drafts and batched delivery (service level and MCP)."""

from mcp import Client

from herdr_inbox import service
from herdr_inbox.server import mcp
from herdr_inbox.service import Caller
from herdr_inbox.store import Store

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r")


def test_group_ask_shares_group_and_dedupes_per_item(store, fake_herdr):
    first = service.ask(store, A, "Already asked?")
    out = service.ask_group(store, A, [{"question": "New one?", "header": "a very long header"},
                                       {"question": "already asked"}, {"question": "Second new?", "multi_select": True}])
    assert [(r["id"], r["status"]) for r in out["questions"]] == [(2, "open"), (first["id"], "already_open"), (3, "open")]
    assert out["group"] == 2 and "Q2 (open), Q1 (already_open), Q3 (open)" in out["message"]
    assert [q.id for q in store.group(2)] == [2, 3]
    assert store.get(2).header == "a very long " and store.get(3).multi_select
    assert store.get(1).group == 1  # the dedupe hit keeps its own group
    assert len(fake_herdr.calls_of("notification", "show")) == 2  # one per call, not per question
    assert fake_herdr.calls_of("notification", "show")[-1][2].startswith("Q2, Q3 ·")


def test_drafts_are_not_sent_and_submit_sends_only_drafted(store, fake_herdr):
    ids = [r["id"] for r in service.ask_group(store, A, [{"question": "One?"}, {"question": "Two?"}])["questions"]]
    assert service.save_draft(store, ids[0], service.make_detail("first"))
    assert store.get(ids[0]).status == "open" and fake_herdr.calls_of("agent", "prompt") == []
    done = service.submit(store, ids)
    assert [q.id for q in done] == [ids[0]] and store.get(ids[1]).status == "open"
    assert fake_herdr.calls_of("agent", "prompt") == [["agent", "prompt", "w1:p1", "[inbox] Q1 answered — One?: first"]]


def test_batched_delivery_one_message_per_pane(store, fake_herdr):
    a1 = service.ask(store, A, "A one?")["id"]
    b1 = service.ask(store, B, "B one?")["id"]
    a2 = service.ask(store, A, "A two?", options=["x", "y"])["id"]
    for qid, d in ((a1, service.make_detail("ok")), (b1, service.make_detail("fine")),
                   (a2, service.make_detail(None, ["y"], {"y": "carefully"}))):
        store.set_draft(qid, d)
    service.submit(store, [a1, b1, a2])
    prompts = {c[2]: c[3] for c in fake_herdr.calls_of("agent", "prompt")}
    assert prompts == {
        "w1:p1": "[inbox] answered — Q1 A one?: ok · Q3 A two?: y (note: carefully)",
        "w1:p2": "[inbox] Q2 answered — B one?: fine",
    }
    assert store.get(a2).answer_json == {"choices": ["y"], "notes": {"y": "carefully"}, "text": None}


def test_batch_respects_session_and_retry_rules(store, fake_herdr, monkeypatch):
    old = service.ask(store, Caller("w1:p1", "w1:t1", "t", "old-conversation", "/r"), "Old?")["id"]
    new = service.ask(store, A, "New?")["id"]
    for qid in (old, new):
        store.set_draft(qid, service.make_detail("ans"))
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.submit(store, [old, new])
    assert "conversation changed" in store.get(old).delivery_error
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "1")
    fake_herdr.reset()
    service.retry_deliveries(store, "w1:p1")
    assert fake_herdr.calls_of("agent", "prompt") == []  # focused: wait
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "0")
    service.retry_deliveries(store, "w1:p1")
    assert fake_herdr.calls_of("agent", "prompt") == [["agent", "prompt", "w1:p1", "[inbox] Q2 answered — New?: ans"]]
    assert store.get(old).delivered_at is None


def test_cli_answer_still_answers_immediately(store, fake_herdr):
    qid = service.ask(store, A, "Now?")["id"]
    q = service.answer(store, qid, "yes")
    assert q.status == "answered" and q.delivered_at


async def test_mcp_questions_parameter(fake_herdr):
    async with Client(mcp) as client:
        r = (await client.call_tool("ask_operator", {"questions": [
            {"question": "DB?", "header": "DB", "options": ["pg", {"label": "sqlite", "description": "one file"}]},
            {"question": "Cache?", "multi_select": True, "options": ["redis", "memcached"]},
        ], "asked_by": "planner"})).structured_content
        assert [q["status"] for q in r["questions"]] == ["open", "open"] and r["group"] == 1
        bad = (await client.call_tool("ask_operator", {"questions": [{"question": f"q{i}?"} for i in range(5)]}))
        assert bad.structured_content["status"] == "error"
        both = (await client.call_tool("ask_operator", {"question": "x?", "questions": [{"question": "y?"}]}))
        assert both.structured_content["status"] == "error"
        many = (await client.call_tool("ask_operator", {"question": "x?", "options": [str(i) for i in range(10)]}))
        assert many.structured_content["status"] == "error"
    with Store() as s:
        q1, q2 = s.get(1), s.get(2)
    assert q1.options[1] == {"label": "sqlite", "description": "one file"} and q1.header == "DB"
    assert q2.multi_select and q2.group == 1 and q1.asked_by == "planner"
