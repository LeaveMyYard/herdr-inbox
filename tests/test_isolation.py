"""Tests never reach a live herdr, even when pytest runs inside a herdr pane."""

import os

from herdr_inbox import herdr

from .conftest import NO_HERDR


def test_no_herdr_environment_leaks_into_a_test():
    assert not [v for v in os.environ if v.startswith("HERDR_") and v != "HERDR_BIN_PATH"]
    assert os.environ["HERDR_BIN_PATH"] == NO_HERDR
    res, err = herdr.call("agent", "list")
    assert res is None and err  # nothing to talk to


def test_the_fake_replaces_it(fake_herdr):
    assert os.environ["HERDR_BIN_PATH"] == str(fake_herdr.bin)
    assert "HERDR_SOCKET_PATH" not in os.environ


def test_a_worker_left_running_is_found_by_the_teardown_guard(fake_herdr, tmp_path):
    """fake_herdr's teardown fails a test whose real `inbox worker` outlives it
    (test_stdio_server_end_to_end once left two per run): the guard finds exactly that one."""
    import subprocess
    import sys
    import time

    from .conftest import leaked_workers

    assert leaked_workers(tmp_path) == []
    (tmp_path / "worker.log").touch()  # what worker.ensure() writes before it starts one
    proc = subprocess.Popen([sys.executable, "-m", "herdr_inbox.cli", "worker"], stdin=subprocess.DEVNULL,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            env={**os.environ, "HERDR_INBOX_WORKER_IDLE_EXIT": "60"})
    try:
        deadline = time.monotonic() + 10
        while leaked_workers(tmp_path) != [proc.pid]:
            assert proc.poll() is None and time.monotonic() < deadline, leaked_workers(tmp_path)
            time.sleep(0.05)
    finally:
        proc.terminate()
        proc.wait(timeout=10)
    assert leaked_workers(tmp_path) == []
