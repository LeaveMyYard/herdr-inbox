"""Old per-version MCP venvs in ${CLAUDE_PLUGIN_DATA} are removed once nothing uses them.

"In use" is answered by the kernel: every MCP server holds a shared flock on
``<venv>/.inbox-in-use`` for its lifetime, and cleanup deletes a venv only while holding an
exclusive non-blocking flock on it -- whatever the path spelling (symlinks, relative paths)
and without asking ``ps``."""

import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

import pytest

from herdr_inbox import venvs

HOUR, DAY = 3600, 86400


def make(data, version, age=2 * HOUR, lock=True):
    d = data / ("venv" if version is None else f"venv-{version}")
    (d / "bin").mkdir(parents=True)
    if lock:
        (d / venvs.LOCK_NAME).touch()
    old = time.time() - age
    os.utime(d, (old, old))
    return d


def holder(venv):
    """A separate process that holds the shared lock, like a running MCP server."""
    proc = subprocess.Popen([sys.executable, "-c", (
        "import fcntl, sys; f = open(sys.argv[1], 'a'); fcntl.flock(f, fcntl.LOCK_SH);"
        " print('held', flush=True); sys.stdin.read()"), str(venv / venvs.LOCK_NAME)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    assert proc.stdout.readline().strip() == "held"
    return proc


def release(proc):
    proc.stdin.close()
    proc.wait(timeout=10)


def test_scope_older_settled_versions_only(tmp_path):
    data = tmp_path / "data"
    old, fresh, current, newer = (make(data, "0.5.1"), make(data, "0.4.9", age=60),
                                  make(data, "0.5.3"), make(data, "0.6.0"))
    odd = [make(data, v) for v in ("0.5.0rc1", "x")]
    (data / "notvenv").mkdir()
    assert venvs.cleanup(data, "0.5.3") == [old]
    assert all(p.exists() for p in (fresh, current, newer, *odd, data / "notvenv"))


def test_a_live_holder_blocks_deletion_and_a_released_lock_allows_it(tmp_path):
    data = tmp_path / "data"
    used = make(data, "0.5.2")
    proc = holder(used)
    try:
        assert venvs.cleanup(data, "0.5.3") == [] and used.exists()
    finally:
        release(proc)
    assert venvs.cleanup(data, "0.5.3") == [used] and not used.exists()


def test_a_symlinked_or_relative_data_dir_changes_nothing(tmp_path, monkeypatch):
    real = tmp_path / "real-data"
    used = make(real, "0.5.2")
    link = tmp_path / "link-data"
    link.symlink_to(real)
    proc = holder(used)  # the server sees the resolved path, cleanup the link
    try:
        assert venvs.cleanup(link, "0.5.3") == []
        monkeypatch.chdir(tmp_path)
        assert venvs.cleanup(link.relative_to(tmp_path), "0.5.3") == [] and used.exists()
    finally:
        release(proc)
    removed = venvs.cleanup(link.relative_to(tmp_path), "0.5.3")
    assert len(removed) == 1 and not used.exists()


def test_venvs_without_a_lock_file_count_as_in_use_for_a_week(tmp_path):
    """Pre-lock servers (0.5.2 and older, the shared `venv`) hold no lock."""
    data = tmp_path / "data"
    recent_shared, recent = make(data, None, age=2 * DAY, lock=False), make(data, "0.5.2", age=2 * DAY, lock=False)
    old = make(data, "0.5.1", age=8 * DAY, lock=False)
    assert venvs.cleanup(data, "0.5.3") == [old]
    assert recent.exists() and recent_shared.exists()


def test_concurrent_cleanups_remove_each_venv_once_without_errors(tmp_path):
    data = tmp_path / "data"
    olds = [make(data, f"0.4.{i}") for i in range(6)]
    with ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(lambda _: venvs.cleanup(data, "0.5.3"), range(16)))
    removed = [p for r in results for p in r]
    assert sorted(removed) == sorted(olds) and not any(p.exists() for p in olds)


def test_the_server_holds_its_own_venv_lock(tmp_path):
    data = tmp_path / "data"
    mine = make(data, "0.5.2", lock=False)
    fh = venvs.hold_lock(mine)
    try:
        assert (mine / venvs.LOCK_NAME).exists()
        assert venvs.cleanup(data, "0.5.3") == [] and mine.exists()
    finally:
        fh.close()
    old = time.time() - 2 * HOUR  # creating the lock file touched the dir; let it settle
    os.utime(mine, (old, old))
    assert venvs.cleanup(data, "0.5.3") == [mine]


def test_the_server_start_call_uses_the_plugin_data_dir_and_this_version(tmp_path, monkeypatch):
    from importlib.metadata import version

    data = tmp_path / "data"
    old = make(data, "0.0.1")
    current = make(data, version("herdr-inbox"))
    monkeypatch.setenv("CLAUDE_PLUGIN_DATA", str(data))
    assert venvs.cleanup_from_env() == [old] and current.exists()
    monkeypatch.delenv("CLAUDE_PLUGIN_DATA")
    assert venvs.cleanup_from_env() == []
