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
