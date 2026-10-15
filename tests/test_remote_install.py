"""`inbox remote install|upgrade <machine>`: both plugins on a saved herdr SSH machine, over
plain ssh (BatchMode, keys only), one printed step at a time, stopping at the first failure."""

import json
import os
import sys
from pathlib import Path

import pytest

from herdr_inbox import cli, remote

MACHINES = [{"id": "abc", "label": "moneylust", "target": "moneylust", "session": "default", "enabled": True},
            {"id": "def", "label": "off", "target": "off-host", "session": "default", "enabled": False},
            {"id": "ghi", "label": "evil", "target": "-oProxyCommand=touch /tmp/pwned", "session": "default",
             "enabled": True}]
BIN = "/home/agent/.local/bin/"


@pytest.fixture
def fake_ssh(tmp_path, monkeypatch, fake_herdr):
    log = tmp_path / "ssh.jsonl"
    script = tmp_path / "ssh"
    script.write_text(f"""#!{sys.executable}
import json, os, sys
args = sys.argv[1:]
with open({str(log)!r}, "a") as fh:
    fh.write(json.dumps(args) + "\\n")
cmd = args[-1]
if "command -v" in cmd:
    print("/home/agent/.local/bin/uv\\n/home/agent/.local/bin/herdr\\n/home/agent/.local/bin/claude")
    sys.exit(0)
fail = os.environ.get("FAKE_SSH_FAIL")
if fail and fail in cmd:
    sys.stderr.write("boom\\n"); sys.exit(1)
if "marketplace add" in cmd and os.environ.get("FAKE_SSH_ALREADY"):
    sys.stderr.write("Marketplace 'herdr-inbox' is already installed\\n"); sys.exit(1)
if "herdr plugin list" in cmd:
    print(json.dumps({{"result": {{"plugins": [{{"plugin_id": "leavemyyard.herdr-inbox", "version": "0.7.0",
                                              "plugin_root": "/home/agent/.config/herdr/plugins/x"}}]}}}}))
    sys.exit(0)
print("ok")
""")
    script.chmod(0o755)
    monkeypatch.setenv("INBOX_SSH", str(script))
    monkeypatch.setattr(remote, "machines", lambda: [m for m in MACHINES if m["enabled"]])
    return log


def calls(log):
    return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []


def test_install_runs_both_plugins_over_batchmode_ssh(fake_ssh, capsys):
    assert cli.main(["remote", "install", "moneylust"]) == 0
    runs = calls(fake_ssh)
    for argv in runs:
        assert "BatchMode=yes" in argv and argv[-3:-1] == ["--", "moneylust"]  # keys only; `--` before the target
    commands = [argv[-1] for argv in runs]
    assert "command -v" in commands[0]  # absolute paths: non-interactive ssh has a minimal PATH
    assert commands[1] == BIN + "claude plugin marketplace add LeaveMyYard/herdr-inbox"
    assert commands[2] == BIN + "claude plugin install herdr-inbox@herdr-inbox"
    assert commands[3] == BIN + "herdr plugin install -y LeaveMyYard/herdr-inbox"
    assert commands[4] == BIN + "herdr plugin list --json"
    out = capsys.readouterr().out
    assert "herdr-inbox 0.7.0 installed on moneylust" in out and "Restart" in out


def test_install_with_a_ref(fake_ssh):
    assert cli.main(["remote", "install", "moneylust", "--ref", "feat/remote-machines"]) == 0
    commands = [argv[-1] for argv in calls(fake_ssh)]
    assert commands[1] == BIN + "claude plugin marketplace add 'LeaveMyYard/herdr-inbox#feat/remote-machines'"
    assert commands[3] == BIN + "herdr plugin install -y --ref feat/remote-machines LeaveMyYard/herdr-inbox"


def test_a_ref_with_shell_characters_is_refused(fake_ssh, capsys):
    assert cli.main(["remote", "install", "moneylust", "--ref", "main; rm -rf ~"]) == 2
    assert not calls(fake_ssh)


def test_a_target_that_looks_like_an_ssh_option_is_refused(fake_ssh):
    """`-oProxyCommand=...` as a target would run a command HERE."""
    assert cli.main(["remote", "install", "evil"]) == 2
    assert not calls(fake_ssh)


def test_an_existing_marketplace_is_fine(fake_ssh, monkeypatch):
    """Claude Code says 'already on disk' / 'already installed' with exit 0 (moneylust)."""
    assert cli.main(["remote", "install", "moneylust"]) == 0


def test_the_first_failing_step_stops_the_install(fake_ssh, monkeypatch, capsys):
    monkeypatch.setenv("FAKE_SSH_FAIL", "claude plugin install")
    assert cli.main(["remote", "install", "moneylust"]) == 1
    assert len(calls(fake_ssh)) == 3  # probe, marketplace, the failing step: nothing after
    assert "boom" in capsys.readouterr().err


def test_upgrade(fake_ssh):
    assert cli.main(["remote", "upgrade", "moneylust"]) == 0
    commands = [argv[-1] for argv in calls(fake_ssh)]
    assert commands[1:4] == [BIN + "claude plugin marketplace update herdr-inbox",
                             BIN + "claude plugin update herdr-inbox@herdr-inbox",
                             BIN + "herdr plugin install -y LeaveMyYard/herdr-inbox"]


def test_unknown_or_disabled_machine(fake_ssh, capsys):
    assert cli.main(["remote", "install", "nope"]) == 2
    assert cli.main(["remote", "install", "off"]) == 2
    assert not calls(fake_ssh)


def test_machines_reads_herdrs_saved_profiles(fake_herdr, monkeypatch, tmp_path):
    fake = tmp_path / "herdr-machines"
    fake.write_text(f"#!/bin/sh\necho '{json.dumps(MACHINES)}'\n")
    fake.chmod(0o755)
    monkeypatch.setenv("HERDR_BIN_PATH", str(fake))
    assert [m["label"] for m in remote.machines()] == ["moneylust", "evil"]  # enabled only


def test_a_ref_starting_with_a_dash_is_refused(fake_ssh):
    assert cli.main(["remote", "install", "moneylust", "--ref=-x"]) == 2
    assert not calls(fake_ssh)


def test_the_probe_runs_under_bin_sh(fake_ssh):
    """ssh runs commands through the login shell (maybe fish): the probe is wrapped."""
    cli.main(["remote", "install", "moneylust"])
    assert calls(fake_ssh)[0][-1].startswith("exec /bin/sh -c ")


def test_only_a_known_already_installed_message_counts_as_success(fake_ssh, monkeypatch):
    monkeypatch.setenv("FAKE_SSH_FAIL", "claude plugin install")
    script = Path(os.environ["INBOX_SSH"])
    text = script.read_text()
    assert "boom" in text
    script.write_text(text.replace("boom", "error: plugin already broken"))
    assert cli.main(["remote", "install", "moneylust"]) == 1
