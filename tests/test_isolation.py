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



def test_a_worker_left_running_is_found_by_the_teardown_guard(fake_herdr, tmp_path, monkeypatch):
    """fake_herdr's teardown fails a test whose real `inbox worker` outlives it
    (test_stdio_server_end_to_end once left two per run): a worker started the real way,
    worker.ensure(), is found by the guard, exactly that one."""
    import signal
    import time

    from herdr_inbox import worker

    from .conftest import leaked_workers, stop_own_workers

    assert leaked_workers(tmp_path) == []
    monkeypatch.delenv("HERDR_INBOX_NO_WORKER")
    monkeypatch.setenv("HERDR_INBOX_WORKER_IDLE_EXIT", "60")
    assert worker.ensure() is True
    found, deadline = [], time.monotonic() + 10
    try:
        while not found:
            assert time.monotonic() < deadline, "the worker ensure() started was never found"
            time.sleep(0.05)
            found = leaked_workers(tmp_path)
        assert len(found) == 1
    finally:
        assert stop_own_workers(tmp_path, signal.SIGTERM) == [], "only this run's worker"
    deadline = time.monotonic() + 10
    while leaked_workers(tmp_path):
        assert time.monotonic() < deadline, "the worker did not stop"
        time.sleep(0.05)
