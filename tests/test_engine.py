"""P5-I: Python's callers start the worker the `engine` file chose. The popup stays on Python
until P6-E and old sessions keep Python MCP servers, so without this whoever asks first would
start a Python worker and quietly undo the switch. scripts/launch is the one decider; these
tests run the real launcher in a scratch checkout."""

import shutil
import subprocess
from pathlib import Path

import pytest

from herdr_inbox import herdr, worker

ROOT = Path(__file__).resolve().parents[1]
REAL_LAUNCHER_BINARY = worker.launcher_binary


def stand_in(path: Path, contract: int = 2) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f'#!/bin/sh\n[ "$1" = contract ] && echo {contract}\nexit 0\n')
    path.chmod(0o755)
    return path


@pytest.fixture
def checkout(tmp_path, fake_herdr, monkeypatch):
    root = tmp_path / "plugin"
    (root / "scripts").mkdir(parents=True)
    for script in ("launch", "release-lib.sh"):
        shutil.copy(ROOT / "scripts" / script, root / "scripts" / script)
    (root / "engine").write_text("rust\n")
    monkeypatch.setattr(herdr, "_own_checkout", lambda: str(root))
    monkeypatch.setattr(worker, "launcher_binary", REAL_LAUNCHER_BINARY)
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path / "state"))
    monkeypatch.delenv("CLAUDE_PLUGIN_DATA", raising=False)
    return root


def spawned_by_ensure(monkeypatch) -> list[list[str]]:
    """``worker.ensure``'s spawn, recorded: run in-process (the fixture's environment), only the
    detached start is replaced, so the launcher itself still runs."""
    real, argvs = subprocess.Popen, []

    def popen(argv, *a, **k):
        if k.get("start_new_session"):
            argvs.append(argv)
            return None
        return real(argv, *a, **k)

    monkeypatch.delenv("HERDR_INBOX_NO_WORKER", raising=False)
    monkeypatch.setattr(worker.subprocess, "Popen", popen)
    assert worker.ensure() is True
    return argvs


def test_ensure_starts_the_binary_the_launcher_picks(checkout, monkeypatch):
    binary = stand_in(checkout / "bin" / "herdr-inbox")
    assert spawned_by_ensure(monkeypatch) == [[str(binary), "worker"]]


def test_ensure_finds_a_recorded_binary_too(checkout, tmp_path, monkeypatch):
    """A Claude-cache copy has no bin/: herdr's build step recorded one for it."""
    binary = stand_in(tmp_path / "state" / "herdr-inbox" / "bin" / "herdr-inbox-1.0.0")
    assert spawned_by_ensure(monkeypatch) == [[str(binary), "worker"]]


def test_with_no_binary_ensure_starts_python(checkout, monkeypatch):
    assert spawned_by_ensure(monkeypatch) == [worker.python_worker_argv(worker._package_src())]


def test_a_binary_of_another_contract_is_no_worker(checkout, monkeypatch):
    stand_in(checkout / "bin" / "herdr-inbox", contract=1)
    assert spawned_by_ensure(monkeypatch) == [worker.python_worker_argv(worker._package_src())]


def test_engine_python_starts_python_with_the_binary_on_disk(checkout, monkeypatch):
    stand_in(checkout / "bin" / "herdr-inbox")
    (checkout / "engine").write_text("python\n")
    assert spawned_by_ensure(monkeypatch) == [worker.python_worker_argv(worker._package_src())]


def test_no_checkout_or_no_launcher_means_python(checkout, monkeypatch):
    (checkout / "scripts" / "launch").unlink()
    assert worker.launcher_binary() is None
    monkeypatch.setattr(herdr, "_own_checkout", lambda: None)
    assert worker.launcher_binary() is None


def test_a_launcher_that_hangs_costs_the_timeout_not_the_worker(checkout, monkeypatch):
    def hang(*a, **k):
        raise subprocess.TimeoutExpired("sh", 5)

    monkeypatch.setattr(worker.subprocess, "run", hang)
    assert worker.launcher_binary() is None
