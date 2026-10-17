"""0.10.0: an IDLE Claude session is woken with its answer by the inbox's asyncRewake
watcher (a Stop hook), without keystrokes. See rewake.py and DESIGN.md."""

import json
import subprocess
import sys
import threading

import pytest

from herdr_inbox import proc, rewake, service, store as st, transcript, worker
from herdr_inbox.service import Caller

from .test_hook import run_hook

A = Caller("w1:p1", "w1:t1", "tab-A", "aaaaaaaa-0000-4000-8000-00000000000a", "/r")

# captured on moneylust (Claude Code 2.1.301, 2026-10-16): the woken text's user record
CAPTURED = ("<task-notification>\n<summary>Stop hook feedback</summary>\n</task-notification>\n<system-reminder>\n"
            "Stop hook blocking error from command \"Stop\": [inbox] Q1 answered — "
            "[E2E TEST] which colour for the rewake test?: amber\n\n</system-reminder>")


@pytest.fixture(autouse=True)
def _env(monkeypatch, fake_herdr):
    monkeypatch.setenv("FAKE_HERDR_SESSION", "aaaaaaaa-0000-4000-8000-00000000000a")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    monkeypatch.setattr(proc, "owning_claude", lambda: None)


@pytest.fixture
def tpath(tmp_path):
    p = tmp_path / "aaaaaaaa-0000-4000-8000-00000000000a.jsonl"
    p.write_text("")
    return p


def pending_answer(store, monkeypatch, question="Which colour?", answer="amber"):
    qid = service.ask(store, A, question)["id"]
    store.set_draft(qid, service.make_detail(answer))
    service.submit(store, [qid], deliver=False)
    return qid


class Clock:
    def __init__(self, on_sleep=None):
        self.t, self.sleeps, self.on_sleep = 0.0, 0, on_sleep

    def __call__(self):
        return self.t

    def sleep(self, s):
        self.t += s
        self.sleeps += 1
        if self.on_sleep:
            self.on_sleep(self.sleeps)


def watch(tpath, clock=None):
    clock = clock or Clock()
    return rewake.run({"session_id": "aaaaaaaa-0000-4000-8000-00000000000a", "transcript_path": str(tpath)}, clock=clock, sleep=clock.sleep)


def test_the_matcher_reads_the_captured_wake_record(tpath):
    tpath.write_text(json.dumps({"type": "user", "timestamp": st.now(), "origin": {"kind": "task-notification"},
                                 "message": {"role": "user", "content": CAPTURED}}) + "\n")
    assert transcript.landed_ids(tpath, [1, 2]) == {1}
    segs = ["[inbox] Q1 answered — [E2E TEST] which colour for the rewake test?: amber"]
    assert transcript.landed_kinds(tpath, {1: (segs, None)}) == {1: {0}}


def test_an_idle_session_is_woken_with_its_answer_once(store, monkeypatch, tpath, fake_herdr):
    qid = pending_answer(store, monkeypatch)
    assert watch(tpath) == f"[inbox] Q{qid} answered — Which colour?: amber"
    q = store.get(qid)
    assert q.delivered_at and q.acknowledged_at is None
    assert store.db.execute("SELECT hook_sent_at FROM question_ext WHERE id = ?", (qid,)).fetchone()[0]
    assert not fake_herdr.calls_of("agent", "prompt")
    assert not rewake.watcher_alive("aaaaaaaa-0000-4000-8000-00000000000a")  # the lock goes with the watcher


def test_a_working_or_blocked_session_is_not_woken_until_it_rests(store, monkeypatch, tpath):
    qid = pending_answer(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")

    def rest(n):
        if n == 3:
            monkeypatch.setenv("FAKE_HERDR_STATUS", "blocked")
        if n == 5:
            monkeypatch.setenv("FAKE_HERDR_STATUS", "done")
    clock = Clock(rest)
    assert watch(tpath, clock) and clock.sleeps == 5
    assert store.get(qid).delivered_at


def test_a_pane_holding_another_conversation_ends_the_watch(store, monkeypatch, tpath):
    qid = pending_answer(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_SESSION", "dddddddd-0000-4000-8000-00000000000d")
    assert watch(tpath) is None and store.get(qid).delivered_at is None


def test_a_newer_watcher_supersedes_the_old_one(store, monkeypatch, tpath):
    clock = Clock(lambda n: n == 2 and rewake._claim_ownership("aaaaaaaa-0000-4000-8000-00000000000a"))
    assert watch(tpath, clock) is None and clock.sleeps == 2


def test_the_watch_ends_with_its_claude_process_and_after_the_max_age(store, monkeypatch, tpath):
    dead = subprocess.Popen([sys.executable, "-c", "pass"])
    dead.wait()
    monkeypatch.setattr(proc, "owning_claude", lambda: dead.pid)
    assert watch(tpath) is None
    monkeypatch.setattr(proc, "owning_claude", lambda: None)
    monkeypatch.setattr(rewake, "WATCH_MAX_SECONDS", 10.0)
    clock = Clock()
    assert watch(tpath, clock) is None and 10.0 <= clock.t <= 11.0


def test_nothing_for_a_subagent_or_outside_herdr(store, monkeypatch, tpath):
    pending_answer(store, monkeypatch)
    assert rewake.run({"session_id": "aaaaaaaa-0000-4000-8000-00000000000a", "agent_id": "a1"}) is None
    monkeypatch.delenv("HERDR_PANE_ID")
    assert rewake.run({"session_id": "aaaaaaaa-0000-4000-8000-00000000000a"}) is None


def test_the_worker_leaves_an_idle_watched_session_to_its_watcher(store, monkeypatch, tpath, fake_herdr):
    qid = pending_answer(store, monkeypatch)
    store.db.execute("INSERT INTO sessions (session_id, watch_seen_at) VALUES ('aaaaaaaa-0000-4000-8000-00000000000a', ?)", (st.now(),))  # its stamp
    store.db.execute("UPDATE questions SET delivery_error = 'pending: busy: the agent is working' WHERE id = ?", (qid,))
    started, release = threading.Event(), threading.Event()

    def held():  # a live watcher: holds the session lock
        lock = open(rewake._dir() / "aaaaaaaa-0000-4000-8000-00000000000a.lock", "a")
        import fcntl

        fcntl.flock(lock, fcntl.LOCK_EX)
        started.set()
        release.wait(10)
        lock.close()
    t = threading.Thread(target=held)
    t.start()
    started.wait(5)
    try:
        assert rewake.watcher_alive("aaaaaaaa-0000-4000-8000-00000000000a")
        worker.tick(store)
        assert service.WATCHER_WAIT in store.get(qid).delivery_error
    finally:
        release.set()
        t.join()
    assert not rewake.watcher_alive("aaaaaaaa-0000-4000-8000-00000000000a")
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    assert not fake_herdr.calls_of("agent", "prompt")
    worker.tick(store)  # no watcher (e.g. past its max age): typed, as before
    assert fake_herdr.calls_of("agent", "prompt")


def test_the_registered_watcher_wakes_through_the_launcher(store, monkeypatch, tpath):
    from .test_hook_launch import HOOKS, ROOT, run_hook as launch

    spec = [h for h in HOOKS["Stop"][0]["hooks"] if h.get("asyncRewake")]
    assert len(spec) == 1 and spec[0]["timeout"] * 1.0 > rewake.WATCH_MAX_SECONDS
    qid = pending_answer(store, monkeypatch)
    proc_ = launch_rewake(ROOT, tpath, spec[0])
    assert proc_.returncode == 2, proc_.stderr  # Claude Code's wake code
    assert proc_.stdout.strip() == f"[inbox] Q{qid} answered — Which colour?: amber" and proc_.stderr == ""


def launch_rewake(root, tpath, spec, path_prefix=None):
    import os

    env = {k: v for k, v in os.environ.items() if not k.startswith(("UV_", "VIRTUAL_ENV"))}
    if path_prefix:
        env["PATH"] = f"{path_prefix}{os.pathsep}{env['PATH']}"
    argv = [spec["command"], *(a.replace("${CLAUDE_PLUGIN_ROOT}", str(root)) for a in spec["args"])]
    return subprocess.run(argv, input=json.dumps({"session_id": "aaaaaaaa-0000-4000-8000-00000000000a", "transcript_path": str(tpath)}),
                          capture_output=True, text=True, timeout=60, env=env, cwd=root)


def test_no_crash_or_stray_exit_code_can_wake_a_session(tmp_path, tpath):
    from .test_hook_launch import HOOKS, ROOT

    spec = [h for h in HOOKS["Stop"][0]["hooks"] if h.get("asyncRewake")][0]
    for code in (1, 2, 4):
        bindir = tmp_path / f"bin{code}"
        bindir.mkdir()
        (bindir / "python3").write_text(f"#!/bin/sh\necho junk\nexit {code}\n")
        (bindir / "python3").chmod(0o755)
        assert launch_rewake(ROOT, tpath, spec, bindir).returncode == 0


def test_a_live_watcher_whose_session_hook_is_stale_does_not_hold_typing_back(store, monkeypatch, tpath, fake_herdr):
    """#26 L3: the idle gate is the same liveness stamp as the busy one, not the lock alone."""
    import fcntl

    qid = pending_answer(store, monkeypatch)
    store.db.execute("UPDATE questions SET delivery_error = 'pending: busy: the agent is working' WHERE id = ?", (qid,))
    store.db.execute("INSERT INTO sessions (session_id, watch_seen_at) VALUES ('aaaaaaaa-0000-4000-8000-00000000000a', ?)",
                     (st.now(-service.HOOK_LIVE - st.timedelta(minutes=1)),))
    lock = open(rewake._dir() / "aaaaaaaa-0000-4000-8000-00000000000a.lock", "a")
    fcntl.flock(lock, fcntl.LOCK_EX)
    try:
        monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
        worker.tick(store)
        assert fake_herdr.calls_of("agent", "prompt")
    finally:
        lock.close()


def test_the_waiting_watcher_stamps_its_own_column_never_the_tool_hooks(store, monkeypatch, tpath):
    """#30 M1: the watcher's liveness is not evidence that PostToolBatch/Stop fire."""
    seen = []
    clock = Clock(lambda n: seen.append(tuple(store.db.execute(
        "SELECT hook_seen_at, watch_seen_at FROM sessions WHERE session_id = 'aaaaaaaa-0000-4000-8000-00000000000a'").fetchone() or ())) or n == 2
        and rewake._claim_ownership("aaaaaaaa-0000-4000-8000-00000000000a"))
    assert watch(tpath, clock) is None
    assert seen[0][0] is None and seen[0][1]


def test_m1_a_live_watcher_with_silent_tool_hooks_does_not_hold_a_working_session(store, monkeypatch, tpath, fake_herdr):
    """#30 M1: a watcher is alive (lock + its stamp) but PostToolBatch/Stop never fire (e.g.
    plugin updated mid-session): a WORKING session's answer gets 0.8's busy typing."""
    import fcntl

    qid = pending_answer(store, monkeypatch)
    store.db.execute("UPDATE questions SET delivery_error = 'pending: busy: the agent is working' WHERE id = ?", (qid,))
    store.db.execute("UPDATE question_ext SET pending_since = ? WHERE id = ?", (st.now(-st.timedelta(minutes=5)), qid))
    store.db.execute("INSERT INTO sessions (session_id, watch_seen_at) VALUES ('aaaaaaaa-0000-4000-8000-00000000000a', ?)", (st.now(),))
    monkeypatch.setenv("FAKE_HERDR_STATUS", "working")
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    lock = open(rewake._dir() / "aaaaaaaa-0000-4000-8000-00000000000a.lock", "a")
    fcntl.flock(lock, fcntl.LOCK_EX)
    try:
        worker.tick(store)
    finally:
        lock.close()
    assert fake_herdr.calls_of("agent", "prompt")


def test_l2_the_matcher_survives_a_reworded_wake_wrapper(tpath):
    """#30 L2: found by our own header, not Claude Code's label wording."""
    reworded = ("<task-notification>\n<summary>Background hook output</summary>\n</task-notification>\n"
                "<system-reminder>\nA background hook (Stop) said:\n[inbox] Q1 answered \u2014 "
                "[E2E TEST] which colour for the rewake test?: amber\n\n</system-reminder>")
    tpath.write_text(json.dumps({"type": "user", "timestamp": st.now(), "origin": {"kind": "task-notification"},
                                 "message": {"role": "user", "content": reworded}}) + "\n")
    segs = ["[inbox] Q1 answered \u2014 [E2E TEST] which colour for the rewake test?: amber"]
    assert transcript.landed_kinds(tpath, {1: (segs, None)}) == {1: {0}}
    # a mention inside the text (not a header) still does not count
    tpath.write_text(json.dumps({"type": "user", "timestamp": st.now(), "message": {"role": "user",
                     "content": "<task-notification>see [inbox] Q9 elsewhere</task-notification>"}}) + "\n")
    assert transcript.landed_ids(tpath, [1]) == set()

def test_m1_a_watch_left_behind_by_clear_ends_without_any_answer(store, monkeypatch, tpath, fake_herdr):
    """#30 M1 (reviewer's repro): the pane now holds eeeeeeee-0000-4000-8000-00000000000e; the old watcher has nothing
    pending, yet it checks the pane on a slow cadence and ends."""
    monkeypatch.setenv("FAKE_HERDR_SESSION", "eeeeeeee-0000-4000-8000-00000000000e")
    clock = Clock()
    assert watch(tpath, clock) is None
    assert rewake.PANE_CHECK_EVERY <= clock.t <= rewake.PANE_CHECK_EVERY + 2 * rewake.WATCH_POLL
    assert len(fake_herdr.calls_of("agent", "get")) == 1  # one herdr look, not one per poll


def test_m1b_a_pane_without_a_session_is_never_woken_and_the_watch_ends(store, monkeypatch, tpath, fake_herdr):
    """#30 M1b: herdr reports the agent but no session: wait, never wake; end after a while."""
    qid = pending_answer(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_SESSION", "")
    clock = Clock()
    assert watch(tpath, clock) is None
    assert rewake.UNKNOWN_SESSION_END <= clock.t <= rewake.UNKNOWN_SESSION_END + 2 * rewake.WATCH_POLL
    assert store.get(qid).delivered_at is None


def test_m1b_the_session_reappearing_resumes_the_watch(store, monkeypatch, tpath):
    qid = pending_answer(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_SESSION", "")
    clock = Clock(lambda n: n == 5 and monkeypatch.setenv("FAKE_HERDR_SESSION", "aaaaaaaa-0000-4000-8000-00000000000a"))
    assert watch(tpath, clock) and store.get(qid).delivered_at


def test_a_watcher_delivery_never_writes_hook_seen_at(store, monkeypatch, tpath):
    """#30 final pass 2 NIT: a wake is not evidence that PostToolBatch/Stop fire."""
    qid = pending_answer(store, monkeypatch)
    assert watch(tpath)
    assert store.get(qid).delivered_at
    row = store.db.execute("SELECT hook_seen_at, watch_seen_at FROM sessions WHERE session_id = 'aaaaaaaa-0000-4000-8000-00000000000a'").fetchone()
    assert row is None or row[0] is None
