"""hooks/run-hook.sh: the hook interpreter is resolved once, cached in CLAUDE_PLUGIN_DATA, then
run directly, so a version-manager shim (pyenv's bash script, 60-75 ms per call) is paid at
most once instead of on every PostToolBatch and Stop (Rust rewrite plan §6, P0b).

Every case must still print the hook's output or nothing, and exit 0 (3 -> 2 for rewake)."""

import json
import os
import shutil
import subprocess
import sys
import threading
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
HOOKS = json.loads((ROOT / "hooks/hooks.json").read_text())["hooks"]
REAL_PY = os.path.realpath(sys.executable)
CACHE = "python-interpreter"


def spec(event: str, index: int = 0) -> dict:
    return HOOKS[event][0]["hooks"][index]


def argv_of(event: str, index: int = 0, root: Path = ROOT, script: Path | None = None) -> list[str]:
    s = spec(event, index)
    args = [a.replace("${CLAUDE_PLUGIN_ROOT}", str(root)) for a in s["args"]]
    if script:
        args = [str(script) if a.endswith("/hooks/run.py") else a for a in args]
    return [s["command"], *args]


def tool_bin(tmp_path: Path, python3: str | None = None) -> str:
    """A PATH holding only what run-hook.sh needs, plus an optional ``python3``."""
    b = tmp_path / "bin"
    b.mkdir(exist_ok=True)
    for tool in ("sh", "mkdir", "mv", "rm"):
        found = shutil.which(tool)
        assert found, tool
        if not (b / tool).exists():
            (b / tool).symlink_to(found)
    if python3 is not None:
        (b / "python3").write_text(python3)
        (b / "python3").chmod(0o755)
    return str(b)


def shim(tmp_path: Path, version: tuple | None = None):
    """A pyenv-like shim: logs each call, then execs the real interpreter. ``version`` makes
    the interpreter report that version to the resolution call (an old pinned Python)."""
    log = tmp_path / "shim.log"
    fake = ""
    if version:
        fake = (f'[ "$2" = "-c" ] && exec "{REAL_PY}" -I -c '
                f'"import sys; sys.version_info = {tuple(version)!r}; exec(sys.argv[1])" "$3"\n')
    return tool_bin(tmp_path, f'#!/bin/sh\necho call >> "{log}"\n{fake}exec "{REAL_PY}" "$@"\n'), log


def calls(log: Path) -> int:
    return len(log.read_text().splitlines()) if log.exists() else 0


def env_for(fake_herdr, path: str, data: Path | None) -> dict:
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("CLAUDE_PLUGIN_", "PYENV_", "VIRTUAL_ENV", "UV_"))}
    env["PATH"] = path
    if data is not None:
        env["CLAUDE_PLUGIN_DATA"] = str(data)
    return env


def run(env: dict, event: str = "SessionStart", argv: list[str] | None = None, payload=None):
    return subprocess.run(argv or argv_of(event), input=json.dumps(payload or {"session_id": "s", "source": "startup"}),
                          capture_output=True, text=True, timeout=60, env=env, cwd=ROOT)


def rules(proc) -> bool:
    return proc.returncode == 0 and "<herdr-inbox>" in json.loads(proc.stdout)["hookSpecificOutput"]["additionalContext"]


def cached(data: Path) -> str:
    return os.path.realpath((data / CACHE).read_text().strip())


def test_every_hook_goes_through_run_hook_sh():
    for entries in HOOKS.values():
        for entry in entries:
            for h in entry["hooks"]:
                assert h["command"] == "sh" and h["args"][0] == "-c"
                assert h["args"][1].startswith('sh "$1" "$2" "$3" 2>/dev/null;') and h["args"][1].endswith("exit 0")
                assert h["args"][2:5] == ["herdr-inbox-hook", "${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.sh",
                                          "${CLAUDE_PLUGIN_ROOT}/hooks/run.py"]


def test_cold_cache_resolves_once_and_writes_the_real_interpreter(tmp_path, fake_herdr):
    path, log = shim(tmp_path)
    data = tmp_path / "data" / "nested"  # not created yet
    p = run(env_for(fake_herdr, path, data))
    assert rules(p) and p.stderr == ""
    assert (data / CACHE).read_text().endswith("\n") and cached(data) == REAL_PY  # not the shim
    assert calls(log) == 1  # the one resolution; the hook itself ran on the cached interpreter
    assert sorted(x.name for x in data.iterdir()) == [CACHE]  # no temp file left


def test_warm_cache_never_touches_python3(tmp_path, fake_herdr):
    path, log = shim(tmp_path)
    data = tmp_path / "data"
    env = env_for(fake_herdr, path, data)
    run(env)
    for event in ("PostToolBatch", "Stop", "SubagentStart", "SessionStart"):
        p = run(env, event)
        assert (p.returncode, p.stderr) == (0, "")
    assert calls(log) == 1


def test_cache_used_even_when_python3_is_broken(tmp_path, fake_herdr):
    data = tmp_path / "data"
    data.mkdir()
    (data / CACHE).write_text(REAL_PY + "\n")
    path = tool_bin(tmp_path, "#!/bin/sh\necho boom >&2\necho leaked\nexit 2\n")
    assert rules(run(env_for(fake_herdr, path, data)))


@pytest.mark.parametrize("stale", ["/nonexistent/bin/python3", "python3", "", "garbage\x01\n", "{dir}",
                                   "{noexec}", "/bin/sh", "{notpython}"])
def test_stale_or_invalid_cache_self_heals(tmp_path, fake_herdr, stale):
    """An interpreter that disappeared (pyenv uninstall), a relative path, junk, a directory, a
    non-executable file or an executable that is not named python*: ignored and replaced."""
    path, log = shim(tmp_path)
    data = tmp_path / "data"
    data.mkdir()
    noexec = tmp_path / "py" / "python3.12"
    noexec.parent.mkdir()
    noexec.write_text("#!/bin/sh\nexit 0\n")
    notpython = tmp_path / "py" / "evil"
    notpython.write_text("#!/bin/sh\necho leaked\nexit 0\n")
    notpython.chmod(0o755)
    (data / CACHE).write_text(stale.format(dir=tmp_path, noexec=noexec, notpython=notpython))
    p = run(env_for(fake_herdr, path, data))
    assert rules(p)
    assert cached(data) == REAL_PY
    run(env_for(fake_herdr, path, data), "PostToolBatch")
    assert calls(log) == 1  # healed: the next hook used the cache


def test_an_interpreter_that_disappears_is_re_resolved(tmp_path, fake_herdr):
    """pyenv uninstall / brew upgrade: the cached path vanishes; the next hook re-resolves."""
    venv = tmp_path / "gone" / "bin"
    venv.mkdir(parents=True)
    (venv / "python3").symlink_to(REAL_PY)
    data = tmp_path / "data"
    data.mkdir()
    (data / CACHE).write_text(f"{venv / 'python3'}\n")
    path, log = shim(tmp_path)
    assert rules(run(env_for(fake_herdr, path, data))) and calls(log) == 0
    shutil.rmtree(tmp_path / "gone")
    assert rules(run(env_for(fake_herdr, path, data))) and calls(log) == 1
    assert cached(data) == REAL_PY


def test_an_old_python_is_never_cached(tmp_path, fake_herdr):
    """pyenv picks the version per directory: a session in a project pinned to Python 3.9
    must not pin every later session onto run.py's uv fallback. Nothing is cached, but the
    resolved interpreter still runs the hook directly (review L5: no second trip through the
    shim). Here it is really new enough, so the rules still print."""
    path, log = shim(tmp_path, version=(3, 9, 6))
    data = tmp_path / "data"
    env = env_for(fake_herdr, path, data)
    for _ in range(2):
        assert rules(run(env))
    assert not (data / CACHE).exists()
    assert calls(log) == 2  # the resolution only, each time


@pytest.mark.skipif(os.geteuid() == 0, reason="root ignores directory permissions")
def test_unwritable_state_dir_still_runs_the_hook(tmp_path, fake_herdr):
    path, log = shim(tmp_path)
    data = tmp_path / "data"
    data.mkdir()
    data.chmod(0o500)
    try:
        for _ in range(2):
            assert rules(run(env_for(fake_herdr, path, data)))
        assert list(data.iterdir()) == []
    finally:
        data.chmod(0o700)
    assert calls(log) == 2  # resolved each time (no cache); the hook itself ran on the real interpreter


def test_state_dir_path_is_a_file(tmp_path, fake_herdr):
    path, _ = shim(tmp_path)
    data = tmp_path / "data"
    data.write_text("x")
    assert rules(run(env_for(fake_herdr, path, data)))
    assert data.read_text() == "x"


def test_cache_path_is_a_directory(tmp_path, fake_herdr):
    """Never write into it (mv would move the temp file inside, one per hook)."""
    path, _ = shim(tmp_path)
    data = tmp_path / "data"
    (data / CACHE).mkdir(parents=True)
    for _ in range(2):
        assert rules(run(env_for(fake_herdr, path, data)))
    assert list((data / CACHE).iterdir()) == [] and sorted(x.name for x in data.iterdir()) == [CACHE]


@pytest.mark.parametrize("with_data", [True, False])
def test_missing_python_is_silent_and_exits_0(tmp_path, fake_herdr, with_data):
    data = tmp_path / "data" if with_data else None
    for event in ("SessionStart", "SubagentStart", "PostToolBatch", "Stop"):
        for index in range(len(HOOKS[event][0]["hooks"])):
            p = run(env_for(fake_herdr, tool_bin(tmp_path), data), argv=argv_of(event, index))
            assert (p.returncode, p.stdout, p.stderr) == (0, "", "")
    assert data is None or not (data / CACHE).exists()


def test_no_plugin_data_env_uses_plain_python3(tmp_path, fake_herdr):
    path, log = shim(tmp_path)
    for event in ("SessionStart", "SubagentStart"):
        assert rules(run(env_for(fake_herdr, path, None), event))
    assert calls(log) == 2  # every hook went through python3, nothing cached anywhere


def test_concurrent_cold_hooks_agree_on_one_whole_cache(tmp_path, fake_herdr):
    path, _ = shim(tmp_path)
    data = tmp_path / "data"
    env = env_for(fake_herdr, path, data)
    results = []

    def one():
        results.append(rules(run(env)))

    threads = [threading.Thread(target=one) for _ in range(16)]
    [t.start() for t in threads]
    [t.join() for t in threads]
    assert results == [True] * 16
    assert [x.name for x in data.iterdir()] == [CACHE]
    assert cached(data) == REAL_PY


@pytest.mark.parametrize("warm", [True, False])
@pytest.mark.parametrize("code,expected", [(3, 2), (0, 0), (1, 0), (2, 0), (127, 0)])
def test_rewake_exit_code_passes_through_the_cache(tmp_path, fake_herdr, warm, code, expected):
    """The rewake wrapper maps exactly run.py's 3 to Claude Code's 2: run-hook.sh execs the
    interpreter, so the status is run.py's own, cold or warm."""
    path, _ = shim(tmp_path)
    data = tmp_path / "data"
    if warm:
        data.mkdir()
        (data / CACHE).write_text(REAL_PY + "\n")
    script = tmp_path / "exit.py"
    script.write_text(f"import sys\nsys.stdin.read()\nprint('out')\nsys.exit({code})\n")
    p = run(env_for(fake_herdr, path, data), argv=argv_of("Stop", 1, script=script))
    assert spec("Stop", 1).get("asyncRewake") is True
    assert (p.returncode, p.stdout) == (expected, "out\n")


def test_the_payload_reaches_run_py_on_a_cold_cache(tmp_path, fake_herdr):
    """The resolution never gets the hook's stdin, even from a shim that reads it: the
    payload is still there for run.py."""
    log = tmp_path / "shim.log"
    path = tool_bin(tmp_path, f'#!/bin/sh\n[ "$2" = "-c" ] && {{ IFS= read -r eaten; printf %s "$eaten" > "{log}"; }}\nexec "{REAL_PY}" "$@"\n')
    script = tmp_path / "echo.py"
    script.write_text("import sys\nsys.stdout.write(sys.stdin.read())\n")
    p = run(env_for(fake_herdr, path, tmp_path / "data"), argv=argv_of("PostToolBatch", script=script),
            payload={"session_id": "payload-check"})
    assert json.loads(p.stdout) == {"session_id": "payload-check"} and (tmp_path / "data" / CACHE).exists()
    assert log.read_text() == ""
