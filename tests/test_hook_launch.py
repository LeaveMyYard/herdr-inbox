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


def run_hook(event: str, root: Path, data: Path, path_prefix: Path | None = None, payload=None):
    spec = HOOKS[event][0]["hooks"][0]
    expand = lambda v: v.replace("${CLAUDE_PLUGIN_ROOT}", str(root)).replace("${CLAUDE_PLUGIN_DATA}", str(data))
    env = {k: v for k, v in os.environ.items() if not k.startswith(("UV_", "VIRTUAL_ENV"))}
    env.update(CLAUDE_PLUGIN_ROOT=str(root), CLAUDE_PLUGIN_DATA=str(data))
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
    args = [a.replace("${CLAUDE_PLUGIN_ROOT}", str(ROOT)) for a in spec["args"]]
    for bad in (args[:-1] + ["no-such-event"], args[:-1]):
        proc = subprocess.run([spec["command"], *bad], input="not json", capture_output=True, text=True, timeout=30)
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
    server = json.loads((ROOT / ".claude-plugin/plugin.json").read_text())["mcpServers"]["herdr-inbox"]
    assert server["env"]["UV_PROJECT_ENVIRONMENT"] == f"${{CLAUDE_PLUGIN_DATA}}/venv-{VERSION}"


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


@pytest.mark.parametrize("source", ["startup", "compact"])
def test_a_locked_inbox_db_still_gets_the_rules_out_fast(tmp_path, fake_herdr, source):
    """M1: the rules are printed first; the pending-question lookup and the transcript
    record each give up quickly on a locked DB and never cost the rules."""
    import sqlite3
    import time

    from herdr_inbox.store import Store

    Store().close()  # schema exists
    lock = sqlite3.connect(os.environ["HERDR_INBOX_DB"], isolation_level=None)
    lock.execute("PRAGMA journal_mode=DELETE")
    lock.execute("BEGIN EXCLUSIVE")
    try:
        spec = HOOKS["SessionStart"][0]["hooks"][0]
        argv = [spec["command"], *(a.replace("${CLAUDE_PLUGIN_ROOT}", str(ROOT)) for a in spec["args"])]
        started = time.monotonic()
        proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        proc.stdin.write(json.dumps({"session_id": "s", "source": source, "transcript_path": "/x/s.jsonl"}))
        proc.stdin.close()
        line = proc.stdout.readline()
        printed = time.monotonic() - started
        assert proc.wait(timeout=10) == 0
    finally:
        lock.execute("ROLLBACK")
        lock.close()
    assert "<herdr-inbox>" in json.loads(line)["hookSpecificOutput"]["additionalContext"]
    assert printed < 1.0, printed  # python start (~0.1-0.2 s) + at most the short lookup timeout


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
