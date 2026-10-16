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

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")

# captured on moneylust (Claude Code 2.1.301, 2026-10-16): the woken text's user record
CAPTURED = ("<task-notification>\n<summary>Stop hook feedback</summary>\n</task-notification>\n<system-reminder>\n"
            "Stop hook blocking error from command \"Stop\": [inbox] Q1 answered — "
            "[E2E TEST] which colour for the rewake test?: amber\n\n</system-reminder>")


@pytest.fixture(autouse=True)
def _env(monkeypatch, fake_herdr):
    monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-A")
    monkeypatch.setenv("FAKE_HERDR_STATUS", "idle")
    monkeypatch.setattr(proc, "owning_claude", lambda: None)


@pytest.fixture
def tpath(tmp_path):
    p = tmp_path / "sess-A.jsonl"
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
    return rewake.run({"session_id": "sess-A", "transcript_path": str(tpath)}, clock=clock, sleep=clock.sleep)


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
    assert not rewake.watcher_alive("sess-A")  # the lock goes with the watcher


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
    monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-OTHER")
    assert watch(tpath) is None and store.get(qid).delivered_at is None


def test_a_newer_watcher_supersedes_the_old_one(store, monkeypatch, tpath):
    clock = Clock(lambda n: n == 2 and rewake._claim_ownership("sess-A"))
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
    assert rewake.run({"session_id": "sess-A", "agent_id": "a1"}) is None
    monkeypatch.delenv("HERDR_PANE_ID")
    assert rewake.run({"session_id": "sess-A"}) is None


def test_the_worker_leaves_an_idle_watched_session_to_its_watcher(store, monkeypatch, tpath, fake_herdr):
    run_hook("post-tool-batch", {"session_id": "sess-A", "transcript_path": str(tpath)})  # the hook is live
    qid = pending_answer(store, monkeypatch)
    store.db.execute("UPDATE questions SET delivery_error = 'pending: busy: the agent is working' WHERE id = ?", (qid,))
    started, release = threading.Event(), threading.Event()

    def held():  # a live watcher: holds the session lock
        lock = open(rewake._dir() / "sess-A.lock", "a")
        import fcntl

        fcntl.flock(lock, fcntl.LOCK_EX)
        started.set()
        release.wait(10)
        lock.close()
    t = threading.Thread(target=held)
    t.start()
    started.wait(5)
    try:
        assert rewake.watcher_alive("sess-A")
        worker.tick(store)
        assert service.WATCHER_WAIT in store.get(qid).delivery_error
    finally:
        release.set()
        t.join()
    assert not rewake.watcher_alive("sess-A")
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
    return subprocess.run(argv, input=json.dumps({"session_id": "sess-A", "transcript_path": str(tpath)}),
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
    store.db.execute("INSERT INTO sessions (session_id, hook_seen_at) VALUES ('sess-A', ?)",
                     (st.now(-service.HOOK_LIVE - st.timedelta(minutes=1)),))
    lock = open(rewake._dir() / "sess-A.lock", "a")
    fcntl.flock(lock, fcntl.LOCK_EX)
    try:
        monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
        worker.tick(store)
        assert fake_herdr.calls_of("agent", "prompt")
    finally:
        lock.close()


def test_the_waiting_watcher_keeps_its_session_hook_live(store, monkeypatch, tpath):
    seen = []
    clock = Clock(lambda n: seen.append(store.db.execute(
        "SELECT hook_seen_at FROM sessions WHERE session_id = 'sess-A'").fetchone()) or n == 2
        and rewake._claim_ownership("sess-A"))
    assert watch(tpath, clock) is None
    assert seen[0] and seen[0][0]


def test_m1_a_watch_left_behind_by_clear_ends_without_any_answer(store, monkeypatch, tpath, fake_herdr):
    """#30 M1 (reviewer's repro): the pane now holds sess-NEW; the old watcher has nothing
    pending, yet it checks the pane on a slow cadence and ends."""
    monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-NEW")
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
    clock = Clock(lambda n: n == 5 and monkeypatch.setenv("FAKE_HERDR_SESSION", "sess-A"))
    assert watch(tpath, clock) and store.get(qid).delivered_at
