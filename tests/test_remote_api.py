"""The remote side: `inbox export` / `inbox apply` (docs/REMOTE.md)."""

import json
import subprocess
import sys

import pytest

from herdr_inbox import cli, remote_api, service
from herdr_inbox.service import Caller
from herdr_inbox.store import Store

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")


def test_every_database_has_a_stable_random_id(store):
    a = remote_api.db_id(store)
    assert len(a) == 32 and remote_api.db_id(Store(store.path)) == a


def test_export_full_then_incremental(store, fake_herdr):
    q1 = service.ask(store, A, "One?", options=["a", "b"])["id"]
    full = remote_api.export(store, {"cursor": None})
    assert full["full"] and full["db_id"] == remote_api.db_id(store) and full["seq"] > 0
    assert [q["id"] for q in full["questions"]] == [q1] and full["ids"] == [q1]
    row = full["questions"][0]
    assert row["qkey"] and row["status"] == "open" and row["options"][0]["label"] == "a"
    cursor = {"db_id": full["db_id"], "seq": full["seq"]}
    assert remote_api.export(store, {"cursor": cursor})["questions"] == []  # nothing changed
    q2 = service.ask(store, A, "Two?")["id"]
    inc = remote_api.export(store, {"cursor": cursor})
    assert not inc["full"] and [q["id"] for q in inc["questions"]] == [q2]


def test_export_with_another_db_id_or_a_pruned_cursor_is_full(store, fake_herdr):
    service.ask(store, A, "One?")
    assert remote_api.export(store, {"cursor": {"db_id": "other", "seq": 1}})["full"]
    store.db.execute("DELETE FROM changes")
    store.db.execute("INSERT INTO changes (qid) VALUES (1)")
    assert remote_api.export(store, {"cursor": {"db_id": remote_api.db_id(store), "seq": 0}})["full"]


def test_export_does_not_reconcile(store, fake_herdr, monkeypatch):
    """No data writes: the reconcile pass (un-see / requeue) never runs from a pull."""
    called = []
    monkeypatch.setattr(Store, "_reconcile_unverified", lambda self: called.append(1))
    assert cli.main(["export"], stdin=json.dumps({"cursor": None})) == 0
    assert called == []


def req(store, qid, action="answer", detail=None, request_id="r1"):
    q = store.get(qid)
    return {"request_id": request_id, "db_id": remote_api.db_id(store), "id": qid,
            "qkey": remote_api.qkey(store, q), "action": action, "detail": detail}


def test_apply_answers_once_and_records_the_request_atomically(store, fake_herdr, monkeypatch):
    spawned = []
    monkeypatch.setattr(remote_api, "spawn_delivery", lambda ids, force=False: spawned.append(ids))
    qid = service.ask(store, A, "Merge?", options=["yes", "no"])["id"]
    r = req(store, qid, detail={"choices": ["yes"]})
    assert remote_api.apply(store, r) == {"outcome": "applied"}
    q = store.get(qid)
    assert q.status == "answered" and q.answer == "yes" and spawned == [[qid]]
    row = store.db.execute("SELECT outcome FROM applied_requests WHERE request_id = 'r1'").fetchone()
    assert row[0] == "applied"
    assert remote_api.apply(store, r) == {"outcome": "already_applied"}  # the lost-reply retry
    assert spawned == [[qid]]


def test_apply_same_answer_without_a_record_is_already_applied(store, fake_herdr, monkeypatch):
    """Applied, then the process died before the reply: an identical answer is ours."""
    monkeypatch.setattr(remote_api, "spawn_delivery", lambda ids, force=False: None)
    qid = service.ask(store, A, "Merge?", options=["yes", "no"])["id"]
    service.answer(store, qid, None, choices=["yes"], deliver=False)
    assert remote_api.apply(store, req(store, qid, detail={"choices": ["yes"]}, request_id="r2"))["outcome"] == "already_applied"
    assert remote_api.apply(store, req(store, qid, detail={"choices": ["no"]}, request_id="r3")) == \
        {"outcome": "rejected", "reason": "answered with yes"}


@pytest.mark.parametrize("change,reason", [
    (lambda r: r.update(db_id="0" * 32), "reset"),
    (lambda r: r.update(qkey="x" * 32), "not that question"),
    (lambda r: r.update(id=999), "not that question"),
])
def test_apply_rejects_the_wrong_question(store, fake_herdr, change, reason):
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    r = req(store, qid, detail={"choices": ["yes"]})
    change(r)
    assert remote_api.apply(store, r) == {"outcome": "rejected", "reason": reason}
    assert store.get(qid).status == "open"


def test_apply_on_a_closed_question(store, fake_herdr):
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    service.close(store, qid, "withdrawn")
    assert remote_api.apply(store, req(store, qid, detail={"choices": ["yes"]})) == \
        {"outcome": "rejected", "reason": "withdrawn"}


def test_apply_dismiss_and_hide(store, fake_herdr):
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    assert remote_api.apply(store, req(store, qid, action="dismiss"))["outcome"] == "applied"
    assert store.get(qid).status == "dismissed"


def test_cli_export_and_apply_read_stdin_write_stdout(store, fake_herdr, capsys, monkeypatch):
    monkeypatch.setattr(remote_api, "spawn_delivery", lambda ids, force=False: None)
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    assert cli.main(["export"], stdin=json.dumps({"cursor": None})) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["questions"][0]["id"] == qid
    assert cli.main(["apply"], stdin=json.dumps(req(store, qid, detail={"choices": ["yes"]}))) == 0
    assert json.loads(capsys.readouterr().out) == {"outcome": "applied"}
    assert cli.main(["apply"], stdin="{not json") == 1


def test_status_tombstones_an_unknown_request_so_a_late_apply_refuses(store, fake_herdr, monkeypatch):
    """Re-review of #21: the original apply may still be running on the remote when the
    post-expiry status query arrives. Whichever commits first wins."""
    monkeypatch.setattr(remote_api, "spawn_delivery", lambda ids, force=False: None)
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    assert remote_api.apply(store, {"request_id": "rs", "action": "status"}) == {"outcome": "expired"}
    late = remote_api.apply(store, req(store, qid, detail={"choices": ["yes"]}, request_id="rs"))
    assert late == {"outcome": "rejected", "reason": "expired"}
    assert store.get(qid).status == "open"


def test_status_reports_an_applied_request(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(remote_api, "spawn_delivery", lambda ids, force=False: None)
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    remote_api.apply(store, req(store, qid, detail={"choices": ["yes"]}, request_id="ra"))
    assert remote_api.apply(store, {"request_id": "ra", "action": "status"}) == \
        {"outcome": "recorded", "state": "applied", "reason": None}
