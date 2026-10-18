"""The Claude Code hooks never exit non-zero, and never depend on uv.

Claude Code gives exit code 2 a meaning of its own. For SessionStart and SubagentStart it
cannot block, but the run counts as a hook error: a "hook error" notice is shown, and our
additionalContext (the inbox rules) never reaches the session. `uv run` exits 2 on its own
failures. That includes two plugin versions sharing ${CLAUDE_PLUGIN_DATA}/venv across an
update: the data dir is kept across updates, and sessions started before the update keep
the old root. So the hooks run stdlib-only with `python3 -I`, behind an `sh` wrapper that
always exits 0. The MCP server keeps uv, with one venv per plugin version.
"""

import json
import os
import shutil
import subprocess
import sys
import tomllib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
HOOKS = json.loads((ROOT / "hooks/hooks.json").read_text())["hooks"]
VERSION = tomllib.loads((ROOT / "pyproject.toml").read_text())["project"]["version"]


def copy_plugin(dest: Path) -> Path:
    files = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard"], cwd=ROOT,
                           capture_output=True, text=True, check=True).stdout.split()
    for rel in files:
        (dest / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / rel, dest / rel)
    return dest


def fake_bin(folder: Path, name: str, code: int, log: Path | None = None) -> None:
    folder.mkdir(parents=True, exist_ok=True)
    record = f'echo "$UV_PROJECT_ENVIRONMENT $@" >> "{log}"\n' if log else ""
    (folder / name).write_text(f"#!/bin/sh\n{record}echo '{name}: simulated failure' >&2\nexit {code}\n")
    (folder / name).chmod(0o755)


_LAUNCH_COPY: list[Path] = []


def launch_root(root: Path) -> Path:
    """``root``, except that the checkout itself is replaced by a copy of its tracked files:
    scripts/launch prefers ROOT/target/release/herdr-inbox (a linked checkout's own build), and
    these tests are about the Python hooks the launcher falls back to (review #70 M1)."""
    if root != ROOT:
        return root
    if not _LAUNCH_COPY:
        import atexit
        import tempfile

        base = Path(tempfile.mkdtemp(prefix="inbox-launch-"))
        atexit.register(shutil.rmtree, base, True)
        _LAUNCH_COPY.append(copy_plugin(base / "plugin"))
    return _LAUNCH_COPY[0]


def hermetic_env(data: Path) -> dict:
    """The environment for a hook through scripts/launch: no binary of the developer's (a
    recorded one under the real state dir, or the plugin data dir) can be picked."""
    env = {k: v for k, v in os.environ.items() if not k.startswith(("UV_", "VIRTUAL_ENV"))}
    env.update(CLAUDE_PLUGIN_DATA=str(data), XDG_STATE_HOME=str(Path(data).parent / "launch-xdg-state"))
    return env


def run_hook(event: str, root: Path, data: Path, path_prefix: Path | None = None, payload=None):
    spec = HOOKS[event][0]["hooks"][0]
    root = launch_root(root)
    expand = lambda v: v.replace("${CLAUDE_PLUGIN_ROOT}", str(root)).replace("${CLAUDE_PLUGIN_DATA}", str(data))
    env = hermetic_env(data)
    env.update(CLAUDE_PLUGIN_ROOT=str(root))
    if path_prefix:
        env["PATH"] = f"{path_prefix}{os.pathsep}{env['PATH']}"
    argv = [spec["command"], *(expand(a) for a in spec.get("args", []))]
    return subprocess.run(argv, input=json.dumps(payload or {"session_id": "s", "source": "startup"}),
                          capture_output=True, text=True, timeout=60, env=env, cwd=root)


EVENTS = [("SessionStart", "SessionStart"), ("SubagentStart", "SubagentStart")]


@pytest.mark.parametrize("event,name", EVENTS)
def test_a_failing_uv_does_not_touch_the_hooks(tmp_path, fake_herdr, event, name):
    """uv exits 2 (as with two versions racing on one venv): the hook still exits 0 with
    its context, because it never runs uv."""
    log = tmp_path / "uv-calls"
    fake_bin(tmp_path / "bin", "uv", 2, log)
    proc = run_hook(event, ROOT, tmp_path / "data", tmp_path / "bin")
    assert proc.returncode == 0, proc.stderr
    assert json.loads(proc.stdout)["hookSpecificOutput"]["hookEventName"] == name
    assert not log.exists()


@pytest.mark.parametrize("event,name", EVENTS)
def test_a_failing_python_still_exits_0(tmp_path, fake_herdr, event, name):
    fake_bin(tmp_path / "bin", "python3", 2)
    proc = run_hook(event, ROOT, tmp_path / "data", tmp_path / "bin")
    assert proc.returncode == 0 and proc.stdout == "" and proc.stderr == ""


def test_hook_crash_and_bad_arguments_exit_0(tmp_path, fake_herdr):
    spec = HOOKS["SessionStart"][0]["hooks"][0]
    args = [a.replace("${CLAUDE_PLUGIN_ROOT}", str(launch_root(ROOT))) for a in spec["args"]]
    for bad in (args[:-1] + ["no-such-event"], args[:-1]):
        proc = subprocess.run([spec["command"], *bad], input="not json", capture_output=True, text=True, timeout=30,
                              env=hermetic_env(tmp_path / "data"))
        assert proc.returncode == 0 and proc.stdout == ""


def test_two_plugin_versions_run_hooks_concurrently(tmp_path, fake_herdr):
    """A session started before an update (old root) and one after (new root) share the
    data dir; their hooks race. None may fail."""
    old = copy_plugin(tmp_path / "cache" / "0.5.2")
    new = copy_plugin(tmp_path / "cache" / VERSION)
    data = tmp_path / "data"
    fake_bin(tmp_path / "bin", "uv", 2)  # whatever uv would do, it is not on the hook path
    jobs = [(event, root) for _ in range(12) for event, _ in EVENTS for root in (old, new)]
    with ThreadPoolExecutor(max_workers=16) as pool:
        results = list(pool.map(lambda j: run_hook(j[0], j[1], data, tmp_path / "bin"), jobs))
    assert [r.returncode for r in results] == [0] * len(jobs)
    assert all(json.loads(r.stdout)["hookSpecificOutput"] for r in results)


def test_mcp_server_venv_is_per_version():
    """Per Claude plugin version: the venv follows plugin.json's version, which the release bot
    bumps (pyproject.toml may lead it by the release in flight, test_manifests)."""
    plugin = json.loads((ROOT / ".claude-plugin/plugin.json").read_text())
    server = plugin["mcpServers"]["herdr-inbox"]
    assert server["env"]["UV_PROJECT_ENVIRONMENT"] == f"${{CLAUDE_PLUGIN_DATA}}/venv-{plugin['version']}"


def test_old_python_falls_back_to_uv_and_still_exits_0(tmp_path, fake_herdr, monkeypatch, capsys):
    """The hooks need Python 3.11 (tomllib). An older python3 (macOS ships 3.9) hands over to
    uv with this version's venv; whatever uv does, the hook exits 0 and prints only a
    successful run's output."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("hook_run", ROOT / "hooks" / "run.py")
    run = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(run)
    log = tmp_path / "uv-calls"
    fake_bin(tmp_path / "bin", "uv", 2, log)
    monkeypatch.setenv("PATH", f"{tmp_path / 'bin'}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.setenv("CLAUDE_PLUGIN_DATA", str(tmp_path / "data"))
    monkeypatch.setattr(run.sys, "version_info", (3, 9, 6))
    monkeypatch.setattr(run.sys, "stdin", __import__("io").StringIO("{}"))
    assert run.main(["session-start"]) == 0
    assert capsys.readouterr().out == ""
    assert log.read_text().startswith(f"{tmp_path / 'data'}/venv-{VERSION} run ")


# -- review of #11 -------------------------------------------------------------------------


# What a locked inbox DB may add to the time the rules take to print: the pending-question
# lookup's own timeout (hook.PENDING_TIMEOUT, 0.2 s) plus slack. A fixed number on purpose:
# a hook that waits longer on the lock fails here, whatever the constant says.
LOCK_BUDGET = 0.5
ROUNDS = 3
LAUNCH_DEADLINE = 15.0  # a hook blocked on the lock for good fails here instead of hanging


def _launch_until_rules(source: str) -> tuple[float, str]:
    """Run the SessionStart hook as Claude Code does; seconds until its first line, and the
    line. A hook that prints nothing within LAUNCH_DEADLINE is killed and the test fails (the
    caller still holds the lock, so a plain readline would wait forever)."""
    import threading
    import time

    spec = HOOKS["SessionStart"][0]["hooks"][0]
    argv = [spec["command"], *(a.replace("${CLAUDE_PLUGIN_ROOT}", str(launch_root(ROOT))) for a in spec["args"])]
    env = hermetic_env(Path(os.environ["HERDR_INBOX_DB"]).parent / "launch-data")
    started = time.monotonic()
    proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
                            start_new_session=True, env=env)
    proc.stdin.write(json.dumps({"session_id": "s", "source": source, "transcript_path": "/x/s.jsonl"}))
    proc.stdin.close()
    got: list = []
    reader = threading.Thread(target=lambda: got.append((proc.stdout.readline(), time.monotonic())), daemon=True)
    reader.start()
    reader.join(LAUNCH_DEADLINE)
    if not got:
        os.killpg(proc.pid, 9)  # the sh wrappers and python: the whole group
        proc.wait()
        pytest.fail(f"the hook printed nothing within {LAUNCH_DEADLINE} s")
    line, at = got[0]
    assert proc.wait(timeout=LAUNCH_DEADLINE) == 0
    proc.stdout.close()
    return at - started, line


@pytest.mark.parametrize("source", ["startup", "compact"])
def test_a_locked_inbox_db_still_gets_the_rules_out_fast(tmp_path, fake_herdr, source):
    """M1: the rules are printed first; the pending-question lookup and the transcript
    record each give up quickly on a locked DB and never cost the rules.

    Measured against the same launch on the same DB, unlocked, interleaved: a slow runner's
    interpreter start-up is in both and cancels out (best of ROUNDS unlocked), and the
    MEDIAN locked launch is held to it + LOCK_BUDGET, so a wait on the lock that happens
    only now and then still fails. Issue #45: on macOS runners sqlite's own busy timeout
    (0.2 s of requested sleeps) held the rules ~0.9 s; the hook now bounds it by wall clock
    (hook._bounded)."""
    import sqlite3

    from herdr_inbox.store import Store

    Store(migrate=True).close()  # schema exists (the worker migrated it)
    free, locked = [], []
    for _ in range(ROUNDS):
        free.append(_launch_until_rules(source)[0])
        lock = sqlite3.connect(os.environ["HERDR_INBOX_DB"], isolation_level=None)
        lock.execute("PRAGMA journal_mode=DELETE")
        lock.execute("BEGIN EXCLUSIVE")
        try:
            printed, line = _launch_until_rules(source)
        finally:
            lock.execute("ROLLBACK")
            lock.close()
        assert "<herdr-inbox>" in json.loads(line)["hookSpecificOutput"]["additionalContext"]
        locked.append(printed)
    assert sorted(locked)[ROUNDS // 2] - min(free) < LOCK_BUDGET, (locked, free)


def test_old_python_fallback_kills_the_whole_group_on_timeout(tmp_path, fake_herdr, monkeypatch):
    """Low: a grandchild holding stdout must not keep the fallback past its timeout."""
    import importlib.util
    import time

    spec = importlib.util.spec_from_file_location("hook_run", ROOT / "hooks" / "run.py")
    run = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(run)
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "uv").write_text("#!/bin/sh\n(sleep 30) &\nsleep 30\n")
    (bin_dir / "uv").chmod(0o755)
    monkeypatch.setenv("PATH", f"{bin_dir}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.setattr(run.sys, "version_info", (3, 9, 6))
    monkeypatch.setattr(run, "FALLBACK_TIMEOUT", 0.5)
    monkeypatch.setattr(run.sys, "stdin", __import__("io").StringIO("{}"))
    started = time.monotonic()
    assert run.main(["session-start"]) == 0
    assert time.monotonic() - started < 5
