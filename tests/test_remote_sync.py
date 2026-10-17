"""The operator's side of remote machines: pull into the mirror, the outbox, the worker."""

import json
import os
import sys
from pathlib import Path

import pytest

from herdr_inbox import remote, remote_api, service, worker
from herdr_inbox.service import Caller
from herdr_inbox.store import Store

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")
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
if os.environ.get("FAKE_SSH_APPLY_DOWN") and sub == "apply":
    sys.stderr.write("Connection reset by peer\\n"); sys.exit(255)
if os.environ.get("FAKE_REMOTE_OLD") and sub in ("export", "apply"):
    sys.stderr.write("inbox: error: argument cmd: invalid choice\\n"); sys.exit(2)
env = dict(os.environ, HERDR_INBOX_DB={str(db)!r}, HERDR_INBOX_NO_WORKER="1")
sys.exit(subprocess.run([sys.executable, "-m", "herdr_inbox.cli", sub], env=env, stdin=sys.stdin).returncode)
""")
    script.chmod(0o755)
    monkeypatch.setenv("INBOX_SSH", str(script))
    monkeypatch.setattr(remote, "machines", lambda: [M])
    store = Store(db, migrate=True)  # the remote machine's worker migrated it
    yield store
    store.close()


def mirror(store):
    return {r["remote_id"]: dict(r) for r in store.db.execute("SELECT * FROM remote_questions").fetchall()}


def shown(store, q):
    """The question as the popup shows it: its data plus the identity it was mirrored under."""
    row = mirror(store)[q]
    return {**json.loads(row["data_json"]), "_db_id": row["db_id"]}


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
    data = shown(store, q)
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
    data = shown(store, q)
    rid = remote.enqueue(store, "m1", data, "answer", {"choices": ["no"]})
    service.answer(remote_db, q, None, choices=["yes"], deliver=False)  # answered on the remote meanwhile
    remote.flush(store, M)  # the mirror still says open: sent, and the remote rejects it
    row = store.db.execute("SELECT state, result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()
    assert tuple(row) == ("rejected", "answered with yes")
    assert any("rejected your answer" in n for n in notes(fake_herdr))


def test_a_recreated_remote_inbox_wipes_the_mirror_and_fails_queued_answers(store, remote_db, fake_herdr, tmp_path):
    q = service.ask(remote_db, A, "Old?", options=["yes"])["id"]
    remote.pull(store, M)
    rid = remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["yes"]})
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
    data = shown(store, q)
    rid = remote.enqueue(store, "m1", data, "answer", {"choices": ["yes"]})
    store.db.execute("UPDATE remote_outbox SET expires_at = '2000-01-01T00:00:00+00:00' WHERE request_id = ?", (rid,))
    remote.flush(store, M)
    assert store.db.execute("SELECT state, result FROM remote_outbox").fetchone()[1] == "expired"
    assert remote_db.get(q).status == "open"


def test_actions_go_fifo_per_question(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    data = shown(store, q)
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
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local?")
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
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local?")
    remote.pull(store, M)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down", "d", "y")
        await pilot.pause()
    assert store.db.execute("SELECT action FROM remote_outbox").fetchone()[0] == "dismiss"


async def test_popup_shows_an_offline_machine_and_a_rejection(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local?")
    remote.pull(store, M)
    rid = remote.enqueue(store, "m1", shown(store, q), "hide")
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
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local?")
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


def test_the_ssh_control_socket_path_stays_short(monkeypatch, tmp_path):
    """moneylust e2e: a ControlPath next to a deep checkout's DB was over the unix limit."""
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/run/user/1000")
    monkeypatch.setattr(remote.os.path, "isdir", lambda p: True)
    opts = remote.control_options()
    path = next(o for o in opts if o.startswith("ControlPath="))[len("ControlPath="):]
    assert len(path) - 2 + 40 <= 100
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/" + "x" * 120)
    assert remote.control_options() == ["-o", "ControlMaster=no"]


def test_a_question_deleted_on_the_remote_leaves_the_mirror_on_the_next_pull(store, remote_db, fake_herdr):
    """moneylust e2e: an incremental pull skipped deleted rows; the mirror kept them 'open'
    until the next full snapshot."""
    q = service.ask(remote_db, A, "Gone soon?")["id"]
    remote.pull(store, M)
    with remote_db.transaction():
        remote_db.db.execute("DELETE FROM question_ext WHERE id = ?", (q,))
        remote_db.db.execute("DELETE FROM questions WHERE id = ?", (q,))
    remote.pull(store, M)
    assert q not in mirror(store)


# -- build review of #21 ---------------------------------------------------------------------


def test_m1_an_answer_to_the_question_on_screen_never_lands_on_a_replacement(store, remote_db, fake_herdr):
    """The reviewer's repro: Q12 'Delete old data?' is open in the popup; the remote inbox is
    reset and a new Q12 'Deploy to prod?' replaces it in the mirror; Enter must not queue the
    answer under the new question's identity."""
    q = service.ask(remote_db, A, "Delete old data?", options=["yes"])["id"]
    remote.pull(store, M)
    (key, m, qs), = remote.open_groups(store)
    shown = qs[0]  # what the popup shows
    store.db.execute("UPDATE remote_questions SET db_id = ?, qkey = ?, data_json = ? WHERE remote_id = ?",
                     ("e" * 32, "b" * 32, json.dumps({**shown, "qkey": "b" * 32, "question": "Deploy to prod?"}), q))
    with pytest.raises(LookupError, match="that question changed"):
        remote.enqueue(store, "m1", shown, "answer", {"choices": ["yes"]})
    assert store.db.execute("SELECT COUNT(*) FROM remote_outbox").fetchone()[0] == 0


def test_bidi_overrides_and_line_separators_are_stripped():
    good = {"id": 1, "group": 1, "qkey": "a" * 32, "status": "open",
            "question": "pay ‮evil‬ ok next end", "options": [{"label": "⁦yes⁩"}]}
    data = remote.validate_export(json.dumps({"format": 1, "db_id": "b" * 32, "seq": 1, "full": True, "ids": [1],
                                              "questions": [good]}))
    row = data["questions"][0]
    assert row["question"] == "pay evil ok next end" and row["options"][0]["label"] == "yes"


def test_one_hung_machine_does_not_delay_the_others(migrated_db, fake_herdr, monkeypatch):
    import threading
    import time as _t

    slow = {**M, "id": "slow", "label": "slow", "target": "slow"}
    fast = {**M, "id": "fast", "label": "fast", "target": "fast"}
    monkeypatch.setattr(remote, "enabled_machines", lambda ttl=60.0: [slow, fast])
    started = {}

    def pull(store, machine):
        started[machine["id"]] = _t.monotonic()
        if machine["id"] == "slow":
            _t.sleep(1.0)
        return False

    monkeypatch.setattr(remote, "pull", pull)
    t0 = _t.monotonic()
    remote.sync_all_parallel()
    assert started["fast"] - t0 < 0.5  # not queued behind the slow one


async def test_a_remote_question_closed_there_becomes_an_inert_placeholder(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Ship?", options=["yes"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local?")
    remote.pull(store, M)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down", "enter")
        await pilot.pause()
        assert app.mode == "remote"
        service.close(remote_db, q, "withdrawn")
        remote.pull(store, M)
        app.poll()
        await pilot.pause()
        from herdr_inbox.tui import GoneView
        assert app.query(GoneView) and not app.query(QuestionView)
        await pilot.press("enter")  # inert: nothing queued
        await pilot.pause()
    assert store.db.execute("SELECT COUNT(*) FROM remote_outbox").fetchone()[0] == 0


# -- final pass of #21 ------------------------------------------------------------------------


def test_m1_queued_actions_expire_even_when_the_machine_is_never_reached_again(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["yes"]})
    store.db.execute("UPDATE remote_outbox SET expires_at = '2000-01-01T00:00:00+00:00'")
    monkeypatch.setenv("FAKE_SSH_DOWN", "1")
    monkeypatch.setattr(remote, "enabled_machines", lambda ttl=60.0: [])  # even removed from herdr
    fake_herdr.reset()
    for _ in range(3):
        remote.expire_sweep(store)
    assert store.db.execute("SELECT state, result FROM remote_outbox").fetchone()[1] == "expired"
    expired = [n for n in notes(fake_herdr) if "expired" in n]
    assert len(expired) == 1  # once


def test_l2_an_expiring_send_that_may_have_landed_is_worded_so(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["yes"]})
    store.db.execute("UPDATE remote_outbox SET expires_at = '2000-01-01T00:00:00+00:00', attempts = 1")
    store.db.execute("UPDATE remote_machines SET last_error = 'No route to host'")  # unreachable now
    fake_herdr.reset()
    remote.expire_sweep(store)
    body = fake_herdr.calls_of("notification", "show")[-1][4]
    assert "may have reached" in body and "answer again" not in body


async def test_m1_the_popup_shows_a_past_expiry_as_expired(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local?")
    remote.pull(store, M)
    remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["yes"]})
    store.db.execute("UPDATE remote_outbox SET expires_at = '2000-01-01T00:00:00+00:00'")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert f"⚠ moneylust:Q{q} answer expired" in app.query_one(GroupListView).body_text()


def test_l5_a_settled_row_is_notified_once_even_with_two_workers(store, remote_db, fake_herdr):
    """Two local workers (one per herdr session) may both finish the same head row."""
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    rid = remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["no"]})
    other = Store(store.path)
    fake_herdr.reset()
    assert remote._settle(store, rid, "rejected", "answered with yes", "moneylust:Q1: rejected", "x") is True
    assert remote._settle(other, rid, "rejected", "answered with yes", "moneylust:Q1: rejected", "x") is False
    assert len(fake_herdr.calls_of("notification", "show")) == 1


def test_m2_an_answer_the_remote_cannot_deliver_shows_here_once(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    service.answer(remote_db, q, None, choices=["yes"], deliver=False)
    remote_db.mark_agent_gone(q, "no live agent in pane w1:p1")
    fake_herdr.reset()
    remote.pull(store, M)
    remote.pull(store, M)
    gone = [n for n in notes(fake_herdr) if "gone" in n or "not delivered" in n]
    assert len(gone) == 1 and f"moneylust:Q{q}" in gone[0]
    assert [i["remote_id"] for i in remote.problem_rows(store)] == [q]


async def test_m2_the_popup_lists_it_and_d_clears_it(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local?")
    remote.pull(store, M)
    service.answer(remote_db, q, None, choices=["yes"], deliver=False)
    remote_db.mark_agent_gone(q, "no live agent in pane w1:p1")
    remote.pull(store, M)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert f"⚠ moneylust:Q{q} answered · agent closed — answer kept for check_answers" in body
        await pilot.press("down", "d")
        await pilot.pause()
    assert remote.problem_rows(store) == []


def test_l1_a_remote_apply_refreshes_the_tab_markers(store, fake_herdr, monkeypatch):
    calls = []
    monkeypatch.setattr(remote_api, "spawn_delivery", lambda ids, force=False: None)
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    q = store.get(qid)
    monkeypatch.setattr(service, "update_markers", lambda s, pane: calls.append(pane))
    r = {"request_id": "r9", "db_id": remote_api.db_id(store), "id": qid, "qkey": remote_api.qkey(store, q),
         "action": "answer", "detail": {"choices": ["yes"]}}
    assert remote_api.apply(store, r)["outcome"] == "applied"
    assert calls == ["w1:p1"]


def test_l4_the_remote_command_needs_no_posix_login_shell():
    rec = {"uv": "/r/bin/uv", "herdr": "/r/bin/herdr", "plugin_root": "/r/root"}
    assert remote.remote_command(rec, "export").startswith("env HERDR_BIN_PATH=/r/bin/herdr /r/bin/uv run ")


def test_l3_a_row_of_another_remote_herdr_session_is_left_to_its_own_worker(store, fake_herdr, monkeypatch):
    """apply -> inbox deliver on the remote: a question asked in another herdr session of that
    machine is not typed (nor counted as 'no agent') here; that session's worker is started."""
    from herdr_inbox import cli, worker

    ensured = []
    monkeypatch.setattr(worker, "ensure", lambda server=None: ensured.append(server) or True)
    qid = service.ask(store, A, "Elsewhere?", options=["yes"])["id"]
    store.set_server(qid, "/run/user/1000/herdr-other.sock")
    service.answer(store, qid, None, choices=["yes"], deliver=False)
    fake_herdr.reset()
    assert cli.main(["deliver", str(qid)]) == 0
    assert not fake_herdr.calls_of("agent", "get") and "/run/user/1000/herdr-other.sock" in ensured
    assert store.get(qid).delivery_attempts == 0


# -- re-review: expiry never applies late; the sending window --------------------------------


def test_after_expiry_only_a_status_query_goes_out_never_a_late_apply(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    rid = remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["yes"]})
    monkeypatch.setenv("FAKE_SSH_APPLY_DOWN", "1")
    remote.flush(store, M)  # transport error: attempts 1, still queued
    assert store.db.execute("SELECT state, attempts FROM remote_outbox").fetchone()[:] == ("queued", 1)
    store.db.execute("UPDATE remote_outbox SET expires_at = '2000-01-01T00:00:00+00:00', next_try_at = NULL")
    monkeypatch.delenv("FAKE_SSH_APPLY_DOWN")
    fake_herdr.reset()
    remote.flush(store, M)  # reachable again, past expiry
    assert remote_db.get(q).status == "open"  # never applied >12 h late
    assert tuple(store.db.execute("SELECT state, result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()) == \
        ("failed", "expired")
    assert any("not sent" in c[4] or "expired" in c[2] for c in fake_herdr.calls_of("notification", "show"))


def test_after_expiry_a_send_that_did_land_is_reported_as_reached(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    data = shown(store, q)
    rid = remote.enqueue(store, "m1", data, "answer", {"choices": ["yes"]})
    monkeypatch.setattr(remote_api, "spawn_delivery", lambda ids, force=False: None)
    row = store.db.execute("SELECT * FROM remote_outbox").fetchone()
    remote_api.apply(remote_db, {"request_id": rid, "db_id": row["db_id"], "id": q, "qkey": row["qkey"],
                                 "action": "answer", "detail": {"choices": ["yes"]}})  # applied, reply lost
    store.db.execute("UPDATE remote_outbox SET attempts = 1, expires_at = '2000-01-01T00:00:00+00:00'")
    fake_herdr.reset()
    remote.flush(store, M)
    assert store.db.execute("SELECT state FROM remote_outbox").fetchone()[0] == "done"
    assert any("after all" in c[2] for c in fake_herdr.calls_of("notification", "show"))


def test_a_row_being_sent_is_left_alone_by_the_sweep_and_d(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    rid = remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["yes"]})
    store.db.execute("UPDATE remote_outbox SET expires_at = '2000-01-01T00:00:00+00:00', sending_at = ?",
                     (remote._iso(remote._now()),))
    remote.expire_sweep(store)
    assert store.db.execute("SELECT state FROM remote_outbox").fetchone()[0] == "queued"
    assert remote.clearable(store, rid) is False  # `d` in the popup
    store.db.execute("UPDATE remote_outbox SET sending_at = '2000-01-01T00:00:00+00:00'")  # a stale marker
    remote.expire_sweep(store)
    assert store.db.execute("SELECT state FROM remote_outbox").fetchone()[0] == "failed"


def test_an_older_remote_that_cannot_answer_status_expires_as_may_have_reached(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["yes"]})
    store.db.execute("UPDATE remote_outbox SET attempts = 1, expires_at = '2000-01-01T00:00:00+00:00'")
    old = remote_api.apply
    monkeypatch.setattr(remote, "call", lambda rec, sub, request: __import__("subprocess").CompletedProcess(
        [], 0, json.dumps({"outcome": "rejected", "reason": "bad request"}), ""))
    fake_herdr.reset()
    remote.flush(store, M)
    remote.flush(store, M)
    assert store.db.execute("SELECT state, result FROM remote_outbox").fetchone()[:] == ("failed", "expired")
    shows = fake_herdr.calls_of("notification", "show")
    assert len(shows) == 1 and "may have reached moneylust" in shows[0][4]


# -- final pass 2 -----------------------------------------------------------------------------


def test_l1_only_terminal_remote_failures_show_or_notify(store, remote_db, fake_herdr):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    service.answer(remote_db, q, None, choices=["yes"], deliver=False)
    remote_db.db.execute("UPDATE questions SET delivery_error = ?, delivery_attempts = 1 WHERE id = ?",
                         (service.NOT_VERIFIED, q))  # the remote worker will retry this
    fake_herdr.reset()
    remote.pull(store, M)
    assert remote.problem_rows(store) == [] and not fake_herdr.calls_of("notification", "show")
    remote_db.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ?", (f"{service.NO_AGENT} w1:p1", q))
    remote.pull(store, M)
    assert remote.problem_rows(store) == []  # it becomes agent-gone at its cap, then it shows
    remote_db.db.execute("UPDATE questions SET delivery_error = ?, delivery_attempts = ? WHERE id = ?",
                         (service.NOT_VERIFIED, service.MAX_UNVERIFIED_ATTEMPTS, q))
    remote.pull(store, M)
    assert [p["remote_id"] for p in remote.problem_rows(store)] == [q]
    assert len(fake_herdr.calls_of("notification", "show")) == 1


def test_l2_the_sync_loop_runs_the_expiry_sweep(migrated_db, fake_herdr, monkeypatch):
    swept = []
    monkeypatch.setattr(remote, "expire_sweep", lambda store: swept.append(1))
    monkeypatch.setattr(remote, "enabled_machines", lambda ttl=60.0: [])
    remote.sync_all_parallel()
    assert swept == [1]


def test_l2_a_reachable_already_sent_expired_row_goes_to_the_status_query(store, remote_db, fake_herdr, monkeypatch):
    q = service.ask(remote_db, A, "Q?", options=["yes"])["id"]
    remote.pull(store, M)
    remote.enqueue(store, "m1", shown(store, q), "answer", {"choices": ["yes"]})
    store.db.execute("UPDATE remote_outbox SET attempts = 1, expires_at = '2000-01-01T00:00:00+00:00'")
    remote.expire_sweep(store)  # reachable and sent before: the sweep leaves it to flush
    assert store.db.execute("SELECT state FROM remote_outbox").fetchone()[0] == "queued"
    sent = []
    real = remote.call
    monkeypatch.setattr(remote, "call", lambda rec, sub, request: sent.append(request) or real(rec, sub, request))
    remote.flush(store, M)
    assert [r["action"] for r in sent] == ["status"]  # never an apply
    assert remote_db.get(q).status == "open"


def ralert(store, aid):
    row = store.db.execute("SELECT * FROM remote_alerts WHERE alert_id = ?", (aid,)).fetchone()
    return {**json.loads(row["data_json"]), "_db_id": row["db_id"], "_status": row["status"]}


def test_a_remote_alert_is_mirrored_notified_once_and_acked_through_the_outbox(store, remote_db, fake_herdr):
    from herdr_inbox import alerts

    aid = alerts.raise_alert(remote_db, A, "[E2E TEST] check disk", "92%", "urgent")["id"]
    fake_herdr.reset()
    assert remote.pull(store, M) and remote.pull(store, M)
    assert [n for n in notes(fake_herdr) if "A" in n] == [f"moneylust · A{aid} · urgent · tab-A"]
    data = ralert(store, aid)
    assert data["title"] == "[E2E TEST] check disk" and data["_status"] == "open"
    rid = remote.enqueue_alert_ack(store, "m1", data)
    remote.pull(store, M)  # the ack is still queued: the mirror keeps it acknowledged
    assert remote.open_alerts(store) == []
    remote.flush(store, M)
    assert tuple(store.db.execute("SELECT state, result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()) \
        == ("done", "applied")
    assert alerts.get(remote_db, aid).status == "acked"
    store.db.execute("UPDATE remote_outbox SET state = 'queued' WHERE request_id = ?", (rid,))
    remote.flush(store, M)  # a lost reply: the same request is a no-op there
    assert store.db.execute("SELECT result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()[0] == "already_applied"


def test_a_withdrawn_remote_alert_leaves_the_popup_and_a_stale_ack_is_refused(store, remote_db, fake_herdr):
    from herdr_inbox import alerts

    aid = alerts.raise_alert(remote_db, A, "[E2E TEST] look")["id"]
    remote.pull(store, M)
    data = ralert(store, aid)
    alerts.withdraw(remote_db, A, aid)
    rid = remote.enqueue_alert_ack(store, "m1", data)
    remote.flush(store, M)
    assert tuple(store.db.execute("SELECT state, result FROM remote_outbox WHERE request_id = ?", (rid,)).fetchone()) \
        == ("rejected", "withdrawn")
    remote.pull(store, M)
    assert remote.open_alerts(store) == []
    with pytest.raises(LookupError):
        remote.enqueue_alert_ack(store, "m1", {**data, "akey": "0" * 32})


def test_hostile_alert_rows_are_dropped():
    good = {"id": 1, "akey": "a" * 32, "status": "open", "title": "t‮x", "severity": "evil", "kind": "x"}
    out = remote.clean_alerts([good, {"id": "1"}, {"id": 2, "akey": "zz", "status": "open"},
                               {"id": 3, "akey": "b" * 32, "status": "boom"}])
    assert len(out) == 1 and out[0]["severity"] == "warning" and out[0]["kind"] == "agent" and "‮" not in out[0]["title"]
    assert remote.clean_alerts(None) is None  # an older remote sends none: the mirror is left alone


async def test_popup_acknowledges_a_remote_alert(store, remote_db, fake_herdr):
    from herdr_inbox import alerts

    aid = alerts.raise_alert(remote_db, A, "[E2E TEST] remote alert")["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local?")
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Local 2?")
    remote.pull(store, M)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert f"⚑ moneylust:A{aid} · warning · [E2E TEST] remote alert" in app.query_one(GroupListView).body_text()
        await pilot.press("up", "enter")
        await pilot.pause()
        assert app.mode == "alert"
        await pilot.press("g")  # remote: never focuses a local pane
        await pilot.press("d")
        await pilot.pause()
        assert app.mode == "list"
    assert store.db.execute("SELECT action, state FROM remote_outbox").fetchone()[:] == ("ack_alert", "queued")
    assert not fake_herdr.calls_of("agent", "focus") and not fake_herdr.calls_of("agent", "prompt")


def test_issue_55_a_machine_whose_name_starts_with_q_is_deep_linked(store, remote_db, fake_herdr, monkeypatch):
    """`inbox open --question qnap:Q12` wrote `nap:Q12` (lstrip("Qq")): the machine was never found."""
    from herdr_inbox import cli, deeplink

    qm = {**M, "label": "qnap"}
    monkeypatch.setattr(remote, "machines", lambda: [qm])
    q = service.ask(remote_db, A, "Remote?", options=["yes"])["id"]
    remote.pull(store, qm)
    monkeypatch.setattr(remote, "enabled_machines", lambda ttl=60.0: [qm])
    assert cli.main(["open", "--question", f"qnap:Q{q}"]) == 0
    value = json.loads(deeplink.request_path().read_text())["value"]
    assert value == f"qnap:{q}"
    assert deeplink.resolve(store, {"kind": "question", "value": value})[0][0] == "remote"
