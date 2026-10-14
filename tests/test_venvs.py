"""Old per-version MCP venvs in ${CLAUDE_PLUGIN_DATA} are removed once nothing uses them."""

import os
import time

from herdr_inbox import venvs


def make(data, version, age=7200):
    d = data / f"venv-{version}"
    (d / "bin").mkdir(parents=True)
    old = time.time() - age
    os.utime(d, (old, old))
    return d


def test_removes_only_older_unused_settled_venvs(tmp_path):
    data = tmp_path / "data"
    old, used, fresh, current, newer = (make(data, "0.5.1"), make(data, "0.5.2"), make(data, "0.4.9", age=60),
                                        make(data, "0.5.3"), make(data, "0.6.0"))
    (data / "venv").mkdir()  # the pre-0.5.3 shared venv: older than every version
    os.utime(data / "venv", (time.time() - 7200,) * 2)
    removed = venvs.cleanup(data, "0.5.3", commands=[f"{used}/bin/python {used}/bin/inbox mcp", "vim"])
    assert sorted(p.name for p in removed) == ["venv", "venv-0.5.1"]
    assert not old.exists() and used.exists() and fresh.exists() and current.exists() and newer.exists()


def test_does_nothing_when_processes_cannot_be_listed(tmp_path):
    data = tmp_path / "data"
    old = make(data, "0.5.1")
    assert venvs.cleanup(data, "0.5.3", commands=None) == [] and old.exists()


def test_the_server_start_call_uses_the_plugin_data_dir_and_this_version(tmp_path, monkeypatch):
    from importlib.metadata import version

    data = tmp_path / "data"
    old = make(data, "0.0.1")
    current = make(data, version("herdr-inbox"))
    monkeypatch.setenv("CLAUDE_PLUGIN_DATA", str(data))
    assert venvs.cleanup_from_env() == [old] and current.exists()
    monkeypatch.delenv("CLAUDE_PLUGIN_DATA")
    assert venvs.cleanup_from_env() == []
