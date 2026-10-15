"""The operator's side of remote machines: pull into the mirror, the outbox, the worker."""

import json
import os
import sys
from pathlib import Path

import pytest

from herdr_inbox import remote, remote_api, service, worker
from herdr_inbox.service import Caller
from herdr_inbox.store import Store

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
M = {"id": "m1", "label": "moneylust", "target": "moneylust", "session": "default", "enabled": True}


@pytest.fixture
def remote_db(tmp_path, fake_herdr, monkeypatch):
    """A second database standing in for the remote; a fake ssh runs the remote CLI on it."""
    db = tmp_path / "remote" / "inbox.db"
    db.parent.mkdir()
    script = tmp_path / "ssh"
    script.write_text(f"""#!{sys.executable}
import os, subprocess, sys
args = sys.argv[1:]
cmd = args[-1]
with open({str(tmp_path / 'ssh.log')!r}, "a") as fh:
    fh.write(cmd + "\\n")
if os.environ.get("FAKE_SSH_DOWN"):
    sys.stderr.write("ssh: connect to host moneylust: No route to host\\n"); sys.exit(255)
if "command -v" in cmd:
    print("/remote/bin/uv\\n/remote/bin/herdr\\n/remote/bin/claude"); sys.exit(0)
if "plugin list --json" in cmd:
    print('{{"result": {{"plugins": [{{"plugin_id": "leavemyyard.herdr-inbox", "plugin_root": "/remote/root"}}]}}}}')
    sys.exit(0)
sub = cmd.rsplit(" ", 1)[-1]
if os.environ.get("FAKE_REMOTE_OLD") and sub in ("export", "apply"):
    sys.stderr.write("inbox: error: argument cmd: invalid choice\\n"); sys.exit(2)
env = dict(os.environ, HERDR_INBOX_DB={str(db)!r}, HERDR_INBOX_NO_WORKER="1")
sys.exit(subprocess.run([sys.executable, "-m", "herdr_inbox.cli", sub], env=env, stdin=sys.stdin).returncode)
""")
    script.chmod(0o755)
    monkeypatch.setenv("INBOX_SSH", str(script))
    monkeypatch.setattr(remote, "machines", lambda: [M])
    store = Store(db)
    yield store
    store.close()


def mirror(store):
    return {r["remote_id"]: dict(r) for r in store.db.execute("SELECT * FROM remote_questions").fetchall()}


def notes(fake_herdr):
    return [c[2] for c in fake_herdr.calls_of("notification", "show")]


def test_pull_mirrors_and_notifies_once(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Remote one?", options=["yes", "no"])["id"]
    fake_herdr.reset()
    assert remote.pull(store, M)
    rows = mirror(store)
    assert list(rows) == [q] and rows[q]["status"] == "open"
    assert json.loads(rows[q]["data_json"])["options"][0]["label"] == "yes"
    assert remote.pull(store, M)  # incremental: nothing new
    assert [n for n in notes(fake_herdr) if n.startswith("moneylust · Q")] == [f"moneylust · Q{q} · tab-A"]
    q2 = service.ask(remote_db, A, "Remote two?")["id"]
    remote.pull(store, M)
    assert set(mirror(store)) == {q, q2}


def test_answer_goes_through_the_outbox_idempotently(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Merge?", options=["yes", "no"])["id"]
    remote.pull(store, M)
    data = json.loads(mirror(store)[q]["data_json"])
    rid = remote.enqueue(store, "m1", data, "answer", {"choices": ["yes"]})
    remote.flush(store, M)
    row = store.db.execute("SELECT state, result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()
    assert tuple(row) == ("done", "applied")
    assert remote_db.get(q).status == "answered" and remote_db.get(q).answer == "yes"
    # the reply was lost: the same request again is a no-op on the remote
    store.db.execute("UPDATE remote_outbox SET state = 'queued' WHERE request_id = ?", (rid,))
    store.db.execute("UPDATE remote_questions SET status = 'open' WHERE remote_id = ?", (q,))
    remote.flush(store, M)
    assert store.db.execute("SELECT result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()[0] == "already_applied"


def test_someone_else_answered_first_is_a_shown_rejection(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Merge?", options=["yes", "no"])["id"]
    remote.pull(store, M)
    data = json.loads(mirror(store)[q]["data_json"])
    rid = remote.enqueue(store, "m1", data, "answer", {"choices": ["no"]})
    service.answer(remote_db, q, None, choices=["yes"], deliver=False)  # answered on the remote meanwhile
    remote.flush(store, M)  # the mirror still says open: sent, and the remote rejects it
    row = store.db.execute("SELECT state, result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()
    assert tuple(row) == ("rejected", "answered with yes")
    assert any("rejected your answer" in n for n in notes(fake_herdr))


def test_a_recreated_remote_inbox_wipes_the_mirror_and_fails_queued_answers(store, remote_db, fake_herdr, tmp_path):
    q = service.ask(remote_db, A, "Old?", options=["yes"])["id"]
    remote.pull(store, M)
    rid = remote.enqueue(store, "m1", json.loads(mirror(store)[q]["data_json"]), "answer", {"choices": ["yes"]})
    remote_db.db.execute("UPDATE meta SET value = ? WHERE key = 'db_id'", ("f" * 32,))  # a new database
    remote.pull(store, M)
    assert store.db.execute("SELECT state, result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()[1] == "reset"
    assert any("inbox was reset" in n for n in notes(fake_herdr))
    assert set(mirror(store)) == {q}  # re-pulled under the new identity
    assert mirror(store)[q]["db_id"] == "f" * 32


def test_offline_backs_off_and_keeps_the_mirror(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?")["id"]
    remote.pull(store, M)
    monkeypatch.setenv("FAKE_SSH_DOWN", "1")
    assert not remote.pull(store, M)
    rec = store.db.execute("SELECT last_error, backoff_until FROM remote_machines").fetchone()
    assert "No route to host" in rec[0] and rec[1]
    assert set(mirror(store)) == {q}
    monkeypatch.delenv("FAKE_SSH_DOWN")
    assert not remote.pull(store, M)  # still backing off: no ssh at all


def test_an_old_remote_is_a_permanent_upgrade_error(store, remote_db, fake_herdr, monkeypatch):
    monkeypatch.setenv("FAKE_REMOTE_OLD", "1")
    assert not remote.pull(store, M)
    assert "upgrade" in store.db.execute("SELECT last_error FROM remote_machines").fetchone()[0]


def test_expired_and_stale_actions_are_not_sent(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    data = json.loads(mirror(store)[q]["data_json"])
    rid = remote.enqueue(store, "m1", data, "answer", {"choices": ["yes"]})
    store.db.execute("UPDATE remote_outbox SET expires_at = '2000-01-01T00:00:00+00:00' WHERE request_id = ?", (rid,))
    remote.flush(store, M)
    assert store.db.execute("SELECT state, result FROM remote_outbox").fetchone()[1] == "expired"
    assert remote_db.get(q).status == "open"


def test_actions_go_fifo_per_question(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    data = json.loads(mirror(store)[q]["data_json"])
    first = remote.enqueue(store, "m1", data, "answer", {"choices": ["yes"]})
    second = remote.enqueue(store, "m1", data, "hide")
    remote.flush(store, M)  # one per question per flush, oldest first
    states = dict(store.db.execute("SELECT request_id, state FROM remote_outbox").fetchall())
    assert states == {first: "done", second: "queued"}


def test_hostile_export_rows_are_dropped():
    good = {"id": 1, "group": 1, "qkey": "a" * 32, "status": "open", "question": "ok?"}
    bad = [{"id": "1"}, {**good, "status": "rm -rf"}, {**good, "qkey": "not hex"}, {**good, "options": ["x"] * 20}]
    raw = json.dumps({"format": 1, "db_id": "b" * 32, "seq": 3, "full": True, "ids": [1],
                      "questions": [good, *bad, {**good, "id": 2, "question": "x\x1b[31m" + "y" * 9000}]})
    data = remote.validate_export(raw)
    assert [r["id"] for r in data["questions"]] == [1, 2]
    assert "\x1b" not in data["questions"][1]["question"] and len(data["questions"][1]["question"]) == 4000
    with pytest.raises(ValueError):
        remote.validate_export("x" * (remote.MAX_EXPORT_BYTES + 1))


def test_the_worker_stays_alive_while_a_remote_is_enabled(fake_herdr, monkeypatch):
    """No local work, one enabled remote: the worker keeps pulling past IDLE_EXIT."""
    monkeypatch.setattr(worker, "IDLE_EXIT", 0.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    monkeypatch.setattr(remote, "enabled_machines", lambda ttl=60.0: [M])
    started, ticks = [], {"n": 0}
    monkeypatch.setattr(worker, "start_remote_thread", lambda: started.append(1))

    def tick(store):
        ticks["n"] += 1
        if ticks["n"] >= 5:
            raise KeyboardInterrupt
        return 0

    monkeypatch.setattr(worker, "tick", tick)
    with pytest.raises(KeyboardInterrupt):
        worker.run()
    assert ticks["n"] == 5 and started == [1]


# -- the popup (reads the local mirror only) --------------------------------------------------

from herdr_inbox.tui import GroupListView, InboxApp, QuestionView  # noqa: E402


async def test_popup_lists_and_answers_a_remote_question(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Ship it?", options=["yes", "no"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "Local?")
    remote.pull(store, M)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert f"moneylust:Q{q} Ship it? · 1 question" in body and "moneylust · tab-A" in body
        await pilot.press("down", "enter")
        await pilot.pause()
        assert app.mode == "remote" and app.query_one(QuestionView).q.question == "Ship it?"
        await pilot.press("enter")  # option 1
        await pilot.pause()
        assert app.mode == "list"
        assert "✓ moneylust · answered · sending to moneylust" in app.query_one(GroupListView).body_text()
        app.poll()  # the queued action arrives as its own row
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert f"⏳ moneylust:Q{q} answer queued · sending to moneylust" in body
    row = store.db.execute("SELECT action, detail_json, state FROM remote_outbox").fetchone()
    assert row[0] == "answer" and json.loads(row[1])["choices"] == ["yes"] and row[2] == "queued"
    assert not fake_herdr.calls_of("agent", "prompt")  # never typed from here


async def test_popup_dismisses_a_remote_group_with_confirmation(store, remote_db, fake_herdr):
    service.ask(remote_db, A, "Drop?")
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "Local?")
    remote.pull(store, M)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down", "d", "y")
        await pilot.pause()
    assert store.db.execute("SELECT action FROM remote_outbox").fetchone()[0] == "dismiss"


async def test_popup_shows_an_offline_machine_and_a_rejection(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "Local?")
    remote.pull(store, M)
    rid = remote.enqueue(store, "m1", json.loads(mirror(store)[q]["data_json"]), "hide")
    store.db.execute("UPDATE remote_outbox SET state = 'rejected', result = 'answered with no'")
    store.db.execute("UPDATE remote_machines SET last_error = 'No route to host'")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert "offline since" in body
        assert f"⚠ moneylust:Q{q} hide rejected: answered with no" in body
        await pilot.press("down", "down", "d")
        await pilot.pause()
    assert store.db.execute("SELECT state FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()[0] == "seen"


async def test_remote_text_is_escaped(store, remote_db, fake_herdr):
    service.ask(remote_db, A, "[b]bold[/b] [link=x]x[/link]?")
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r"), "Local?")
    remote.pull(store, M)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert "[b]bold[/b]" in app.query_one(GroupListView).body_text()


async def test_deep_link_to_a_remote_question(store, remote_db, fake_herdr, monkeypatch):
    from herdr_inbox import deeplink

    q = service.ask(remote_db, A, "Remote?", options=["yes"])["id"]
    remote.pull(store, M)
    monkeypatch.setattr(remote, "enabled_machines", lambda ttl=60.0: [M])
    assert deeplink.resolve(store, {"kind": "question", "value": f"moneylust:Q{q}"})[0][0] == "remote"
    assert deeplink.resolve(store, {"kind": "question", "value": f"elsewhere:Q{q}"}) is None
    monkeypatch.setenv("INBOX_OPEN_NONCE", deeplink.write_request("question", f"moneylust:Q{q}", hold=False))
    app = InboxApp(store, focus_pane="w9:p9")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "remote" and app.query_one(QuestionView).q.question == "Remote?"
