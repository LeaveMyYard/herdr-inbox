"""Nothing the operator or an agent does waits for herdr, tab labels or delivery.

Operator report: "sometimes sending herdr-inbox messages blocks all inputs and everything
just lags". Measured on an isolated herdr (PR description): the popup stayed open until the
send finished (the 15 s transcript check when an answer does not land), and an
ask_operator call waited 4 s for the tab-label lock held by another process."""

import os
import subprocess
import sys
import threading
import time

import pytest

from herdr_inbox import herdr, service, tui
from herdr_inbox.service import Caller
from herdr_inbox.tui import InboxApp

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")


@pytest.fixture
def background(monkeypatch):
    monkeypatch.setattr(service, "_bg_queue", None)
    service.enable_background()
    yield
    service.drain(5)
    monkeypatch.setattr(service, "_bg_queue", None)


async def test_popup_records_hands_delivery_off_and_closes_at_once(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(tui, "DETACHED_SEND", True)
    spawned = []
    monkeypatch.setattr(tui, "spawn_delivery", lambda qids, force=False: spawned.append((qids, force)))
    qid = service.ask(store, A, "Merge?", options=["yes", "no"])["id"]
    app = InboxApp(store, focus_pane="w1:p2")  # the asking pane is not the one it returns to
    async with app.run_test() as pilot:
        await pilot.pause()
        t = time.monotonic()
        await pilot.press("enter")
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert not app.is_running and time.monotonic() - t < 1.0
    assert store.get(qid).status == "answered" and store.get(qid).answer == "yes"
    assert not fake_herdr.calls_of("agent", "prompt")  # nothing typed by the popup itself
    assert spawned == [([qid], False)]


async def test_send_now_hands_off_with_force(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(tui, "DETACHED_SEND", True)
    spawned = []
    monkeypatch.setattr(tui, "spawn_delivery", lambda qids, force=False: spawned.append((qids, force)))
    qid = service.ask(store, A, "Late?", options=["yes"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Other?")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.answer(store, qid, "yes")  # pending: agent busy
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down", "enter")
        await pilot.pause()
        await pilot.press("1")  # Send now
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert spawned == [([qid], True)]


def test_cli_deliver_delivers_recorded_answers(store, fake_herdr):
    from herdr_inbox import cli

    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    service.answer(store, qid, "yes", deliver=False)
    assert not fake_herdr.calls_of("agent", "prompt")
    assert cli.main(["deliver", str(qid)]) == 0
    assert store.get(qid).delivered_at and fake_herdr.calls_of("agent", "prompt")


def test_ask_does_not_wait_for_the_tab_label_lock(store, fake_herdr, background, monkeypatch, tmp_path):
    """Measured before: 4065 ms with the lock held by another process."""
    lock = str(store.path) + ".markers.lock"
    holder = subprocess.Popen([sys.executable, "-c", (
        "import fcntl,sys,time; f=open(sys.argv[1],'a+'); fcntl.flock(f, fcntl.LOCK_EX);"
        " print('x', flush=True); time.sleep(3)"), lock], stdout=subprocess.PIPE, text=True)
    holder.stdout.readline()
    try:
        t = time.monotonic()
        service.ask(store, A, "Quick?", options=["a"])
        assert time.monotonic() - t < 0.5
    finally:
        holder.wait()


def test_ask_notification_runs_in_the_background(store, fake_herdr, background, monkeypatch):
    threads = []
    monkeypatch.setattr(herdr, "notify", lambda *a: threads.append(threading.current_thread().name))
    service.ask(store, A, "Toast?")
    service.drain(5)
    assert threads == ["inbox-background"]


async def test_popup_never_calls_herdr_for_tab_labels_on_the_ui_thread(store, fake_herdr, background, monkeypatch):
    monkeypatch.setattr(tui, "LIVE_TAB_LABELS", True)
    calls = []
    monkeypatch.setattr(herdr, "tab_label", lambda tab: calls.append(threading.current_thread().name) or "Live")
    service.ask(store, A, "Where?")
    q = store.get(1)
    assert tui.tab_name(q, store) == "tab-A"  # the stored label at once
    service.drain(5)
    assert tui.tab_name(q, store) == "Live" and calls == ["inbox-background"]


async def test_go_to_agent_focuses_off_the_ui_thread(store, fake_herdr, monkeypatch):
    threads = []
    real = herdr.focus_agent
    monkeypatch.setattr(herdr, "focus_agent", lambda p: threads.append(threading.current_thread().name) or real(p))
    service.ask(store, A, "q1?")
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "q2?")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("g")
        await fake_herdr.went_to_agent(app)
    assert threads and threads[0] != threading.main_thread().name
    assert fake_herdr.calls_of("agent", "focus") == [["agent", "focus", "w1:p1"]]


def test_spawn_delivery_runs_inbox_deliver_detached(fake_herdr, monkeypatch):
    seen = []
    monkeypatch.setattr(tui.subprocess, "Popen", lambda args, **kw: seen.append((args, kw)))
    monkeypatch.setenv("UV_PROJECT_ENVIRONMENT", "/x/venv-0.6.2")
    tui.spawn_delivery([7, 8], force=True)
    args, kw = seen[0]
    assert args[1:] == ["-m", "herdr_inbox.cli", "deliver", "7", "8", "--force"]
    assert kw["start_new_session"] and "UV_PROJECT_ENVIRONMENT" not in kw["env"]


def test_inbox_trace_logs_steps_only_when_set(store, fake_herdr, monkeypatch, tmp_path):
    import json

    log = tmp_path / "trace.jsonl"
    service.ask(store, A, "No trace?")
    assert not log.exists()
    monkeypatch.setenv("INBOX_TRACE", str(log))
    service.ask(store, A, "Traced?")
    steps = [json.loads(line) for line in log.read_text().splitlines()]
    assert any(s["step"] == "herdr notification show" and "ms" in s and s["thread"] for s in steps)


# -- review of #19 ---------------------------------------------------------------------------

import json as _json
from pathlib import Path as _Path


@pytest.fixture
def live_session(fake_herdr):
    path = _Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-fake-project" / "aaaaaaaa-0000-4000-8000-00000000000a.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(_json.dumps({"type": "user", "message": {"content": "hello"}}) + "\n")
    return path


def test_m1_a_stale_claim_takeover_looks_at_the_transcript_first(store, fake_herdr, live_session, monkeypatch):
    """The reviewer's repro: a sender died mid-verify after typing; its claim went stale."""
    qid = service.ask(store, A, "late?", options=["yes"])["id"]
    service.answer(store, qid, "yes", deliver=False)
    with open(live_session, "a") as fh:  # the dead sender's prompt did land
        fh.write(_json.dumps({"type": "user", "message": {"content": f"[inbox] Q{qid} answered — late?: yes"}}) + "\n")
    store.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ?",
                     ("sending:2000-01-01T00:00:00+00:00:deadbeef", qid))
    service.retry_deliveries(store, "w1:p1")
    assert not fake_herdr.calls_of("agent", "prompt") and store.get(qid).verified_at


def test_the_detached_sender_end_to_end(store, fake_herdr, live_session):
    """No mocks: record -> spawn -> the child `inbox deliver` delivers and verifies, once."""
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    service.answer(store, qid, "yes", deliver=False)
    tui.spawn_delivery([qid])
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline and not store.get(qid).verified_at:
        time.sleep(0.1)
    assert store.get(qid).verified_at
    assert len(fake_herdr.calls_of("agent", "prompt")) == 1


def test_a_hidden_pending_answer_is_still_delivered(store, fake_herdr, live_session, monkeypatch):
    """Hiding only removes the row from the popup; only agent-gone rows stop being retried."""
    qid = service.ask(store, A, "Busy?", options=["yes"])["id"]
    monkeypatch.setenv("FAKE_HERDR_STATUS", "blocked")
    service.answer(store, qid, "yes")
    store.hide_failed(qid)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    service.retry_deliveries(store, "w1:p1")
    assert store.get(qid).verified_at


async def test_the_worker_is_ensured_before_the_sender_is_spawned(store, fake_herdr, monkeypatch):
    """A popup killed between record and spawn must not strand the answer."""
    from herdr_inbox import worker

    order = []
    monkeypatch.setattr(tui, "DETACHED_SEND", True)
    monkeypatch.setattr(worker, "ensure", lambda *a: order.append("ensure") or True)
    monkeypatch.setattr(tui, "spawn_delivery", lambda qids, force=False: order.append("spawn"))
    service.ask(store, A, "Merge?", options=["yes", "no"])
    app = InboxApp(store, focus_pane="w1:p2")  # the asking pane is not the one it returns to
    async with app.run_test() as pilot:
        await pilot.pause()
        await app.workers.wait_for_complete()
        order.clear()  # the popup's own start-up ensure does not count
        await pilot.press("enter")
        await app.workers.wait_for_complete()
    assert order == ["ensure", "spawn"]


def test_a_failing_background_job_does_not_stop_later_ones(fake_herdr, background):
    ran = []

    def boom():
        raise SystemExit("a BaseException from a job")

    service.later(boom)
    service.later(lambda: ran.append(1))
    assert service.drain(5) and ran == [1]


# -- re-review of #19 ------------------------------------------------------------------------


def test_an_id_mentioned_inside_another_answer_is_not_a_delivery(store, fake_herdr, live_session):
    """Q28 written inside Q27's answer (' · Q28 …') must not make Q28 count as landed."""
    for i in range(27):
        service.ask(store, A, f"filler {i}?")
    q28 = service.ask(store, A, "Lost?", options=["yes"])["id"]
    assert q28 == 28
    with open(live_session, "a") as fh:
        fh.write(_json.dumps({"type": "user", "message": {"content":
                 "[inbox] answered — Q26 filler 25?: ok · Q27 which region?: eu · Q28 can wait"}}) + "\n")
    service.answer(store, q28, "yes")  # first delivery: the transcript check runs first
    assert len(fake_herdr.calls_of("agent", "prompt")) == 1 and store.get(q28).verified_at


def test_a_record_older_than_the_answer_does_not_count(store, fake_herdr, live_session):
    """inbox.db recreated: ids restart, an old transcript record names the same Q id."""
    with open(live_session, "a") as fh:
        fh.write(_json.dumps({"type": "user", "timestamp": "2026-01-01T00:00:00.000Z",
                              "message": {"content": "[inbox] Q1 answered — Merge?: yes"}}) + "\n")
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    assert qid == 1
    service.answer(store, qid, "yes")
    assert len(fake_herdr.calls_of("agent", "prompt")) == 1 and store.get(qid).verified_at


def test_a_landed_multiline_answer_is_recognised_as_typed(store, fake_herdr, live_session):
    """herdr.clean() turns tabs/newlines/U+2028 into spaces before typing: the landed record
    carries that text, and it must count (else it is retyped up to the cap)."""
    qid = service.ask(store, A, "Pick\tone?", options=["x"])["id"]
    service.answer(store, qid, "a\tb\nc d  e", deliver=False)
    typed = herdr.clean(service.delivery_text([store.get(qid)]))  # exactly what herdr types
    with open(live_session, "a") as fh:  # it landed; the sender died before recording it
        fh.write(_json.dumps({"type": "user", "message": {"content": typed}}) + "\n")
    service.deliver(store, store.get(qid))
    assert not fake_herdr.calls_of("agent", "prompt") and store.get(qid).verified_at


# -- final pass of #19 -----------------------------------------------------------------------


def test_h1_a_landed_long_answer_reference_stays_unseen(store, fake_herdr, live_session):
    """Only the pointer to check_answers landed: verified, but NOT acknowledged."""
    qid = service.ask(store, A, "Long?", options=["x"])["id"]
    service.answer(store, qid, "y" * 4050, deliver=False)
    ref = herdr.clean(service._reference(store.get(qid)))
    with open(live_session, "a") as fh:
        fh.write(_json.dumps({"type": "user", "message": {"content": ref}}) + "\n")
    service.deliver(store, store.get(qid))
    q = store.get(qid)
    assert not fake_herdr.calls_of("agent", "prompt") and q.verified_at and q.acknowledged_at is None
    assert [a.id for a in store.unacknowledged_answers("aaaaaaaa-0000-4000-8000-00000000000a", "w1:p1")] == [qid]


def test_m1_inbox_deliver_keeps_out_of_the_pane_the_operator_is_in(store, fake_herdr, live_session, monkeypatch):
    from herdr_inbox import cli

    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    service.answer(store, qid, "yes", deliver=False)
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "1")
    assert cli.main(["deliver", str(qid)]) == 0
    assert not fake_herdr.calls_of("agent", "prompt") and "operator" in store.get(qid).delivery_error
    assert cli.main(["deliver", str(qid), "--force"]) == 0  # Send now overrides
    assert store.get(qid).verified_at


async def test_m1_the_return_pane_is_typed_while_the_popup_still_holds_the_keyboard(store, fake_herdr, live_session, monkeypatch):
    """The answer to the agent the popup returns to is typed before the popup closes (the
    keyboard is still the popup's); only its verification is handed off."""
    monkeypatch.setattr(tui, "DETACHED_SEND", True)
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "1")  # the operator came from that pane
    verify, deliver = [], []
    monkeypatch.setattr(tui, "spawn_verify", lambda job: verify.append(job))
    monkeypatch.setattr(tui, "spawn_delivery", lambda qids, force=False: deliver.append(qids))
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter")
        await app.workers.wait_for_complete()
        typed_while_open = len(fake_herdr.calls_of("agent", "prompt"))
    assert typed_while_open == 1 and deliver == []
    assert verify and verify[0]["rows"][0]["id"] == qid


def test_m1_inbox_verify_records_the_outcome_with_the_claim(store, fake_herdr, live_session):
    from herdr_inbox import cli

    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    service.answer(store, qid, "yes", deliver=False)
    claim = store.claim_delivery(qid)
    offset = transcript_size = live_session.stat().st_size
    with open(live_session, "a") as fh:
        fh.write(_json.dumps({"type": "user", "message": {"content": service.delivery_text([store.get(qid)])}}) + "\n")
    job = {"path": str(live_session), "offset": offset, "ack": True, "rows": [{"id": qid, "claim": claim}]}
    assert cli.main(["verify", _json.dumps(job)]) == 0
    q = store.get(qid)
    assert q.verified_at and q.acknowledged_at and q.delivered_at


async def test_l1_the_popup_drains_background_jobs_before_exit(store, fake_herdr, monkeypatch):
    drained = []
    monkeypatch.setattr(service, "drain", lambda timeout: drained.append(timeout) or True)
    service.ask(store, A, "q?")
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "q2?")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
    assert drained and max(drained) <= 1.0


def test_l1_the_mcp_server_drains_on_exit(monkeypatch):
    from herdr_inbox import server

    registered = []
    monkeypatch.setattr(server.atexit, "register", lambda fn, *a: registered.append((fn, a)))
    monkeypatch.setattr(service, "_bg_queue", None)
    monkeypatch.setattr(server, "_ensure_worker", lambda: None)
    server.prepare()
    try:
        assert any(fn is service.drain and a and a[0] <= 1.0 for fn, a in registered)
    finally:
        service.drain(5)
        monkeypatch.setattr(service, "_bg_queue", None)


def test_l2_mark_agent_gone_on_a_never_attempted_row(store, fake_herdr):
    qid = service.ask(store, A, "Never sent?", options=["yes"])["id"]
    store.set_answer(qid, "yes")
    assert store.get(qid).delivery_error is None
    store.mark_agent_gone(qid, "no live agent in pane w1:p1")
    assert store.get(qid).delivery_error.startswith("agent gone:")


async def test_l3_send_now_says_when_the_answer_is_already_being_delivered(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(tui, "DETACHED_SEND", True)
    spawned, notes = [], []
    monkeypatch.setattr(tui, "spawn_delivery", lambda qids, force=False: spawned.append(qids))
    qid = service.ask(store, A, "Late?", options=["yes"])["id"]
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r"), "Other?")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    service.answer(store, qid, "yes")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("down", "enter")
        await pilot.pause()
        assert store.claim_delivery(qid)  # another process is sending it right now
        app.notify = lambda message, **kw: notes.append(message)
        await pilot.press("1")
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert spawned == [] and any("already being delivered" in n for n in notes)


# -- cycle reviewer's Lows on a3a35e5 -------------------------------------------------------


def test_a_reopened_store_skips_schema_work_and_tools_skip_reconcile(fake_herdr, tmp_path, monkeypatch):
    """First ask_operator calls on a fresh MCP server took 0.8-1.4 s (reviewer): every tool
    call re-ran the schema script and the reconcile pass, contending with the worker."""
    from herdr_inbox import server
    from herdr_inbox.store import Store

    Store(migrate=True).close()  # the worker migrated: the schema exists now (and is stamped)
    statements = []
    real_connect = __import__("sqlite3").connect

    def connect(*a, **k):
        db = real_connect(*a, **k)
        db.set_trace_callback(statements.append)
        return db

    monkeypatch.setattr("herdr_inbox.store.sqlite3.connect", connect)
    reconciled = []
    monkeypatch.setattr(Store, "_reconcile_unverified", lambda self: reconciled.append(1))
    server.tool_store().close()
    assert not [s for s in statements if s.lstrip().upper().startswith(("CREATE", "ALTER"))]
    assert reconciled == []  # the worker reconciles; a tool call does not


def test_a_failing_verify_spawn_does_not_break_the_send(store, fake_herdr, live_session, monkeypatch):
    """Like the other spawn: an OSError starting `inbox verify` is logged; the claim goes
    stale and the worker (looking at the transcript first) takes the row over."""
    class NoProcesses:
        DEVNULL = -3

        @staticmethod
        def Popen(*a, **k):
            raise OSError("fork failed")

    monkeypatch.setattr(tui, "subprocess", NoProcesses)
    qid = service.ask(store, A, "Merge?", options=["yes"])["id"]
    service.answer(store, qid, "yes", deliver=False)
    service.deliver_batch(store, [store.get(qid)], verify_later=tui.spawn_verify)  # must not raise
    assert len(fake_herdr.calls_of("agent", "prompt")) == 1
    assert store.get(qid).delivery_error.startswith("sending:")  # left for a stale takeover


async def test_waiting_for_the_operator_to_leave_the_pane_says_so(store, fake_herdr, monkeypatch):
    qid = service.ask(store, A, "Busy?", options=["yes"])["id"]
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    service.answer(store, qid, "yes")
    monkeypatch.delenv("FAKE_HERDR_FAIL")
    monkeypatch.setenv("FAKE_HERDR_FOCUSED", "1")
    service.retry_deliveries(store, "w1:p1")
    assert tui.pending_text(store.get(qid).delivery_error[len("pending: "):], 0) == \
        "waiting for you to leave that pane (it is focused)"


# -- schema stamp: concurrent migration (the revision rules: test_schema_migrations.py) ------


def test_seven_processes_migrating_an_unstamped_db_at_once(fake_herdr, tmp_path):
    """The upgrade: ~7 migrating processes (worker and popups, of several herdr servers) open a
    0.6.2 (unstamped) database together."""
    from herdr_inbox.store import Store, revision_stamp

    db = tmp_path / "old.db"
    s = Store(db, migrate=True)
    s.db.execute("PRAGMA user_version = 0")  # as 0.6.2 left it
    s.db.execute("ALTER TABLE question_ext DROP COLUMN agent_gone_at")  # a column added since
    s.close()
    code = "import sys; from herdr_inbox.store import Store; Store(sys.argv[1], migrate=True).close(); print('ok')"
    procs = [subprocess.Popen([sys.executable, "-c", code, str(db)], stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True) for _ in range(7)]
    results = [p.communicate(timeout=60) for p in procs]
    assert [p.returncode for p in procs] == [0] * 7, [r[1][-400:] for r in results]
    s = Store(db)
    assert s.db.execute("PRAGMA user_version").fetchone()[0] == revision_stamp()
    assert "agent_gone_at" in {r[1] for r in s.db.execute("PRAGMA table_info(question_ext)")}

