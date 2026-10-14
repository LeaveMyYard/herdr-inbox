"""The Claude Code plugin must work from the installed copy.

Claude Code installs this plugin by copying the checkout into
~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/, git-ignored files
included but without the virtualenv's interpreter symlinks. A `.venv` like that
breaks any `uv run --project ${CLAUDE_PLUGIN_ROOT}` that uses it. These tests
build such a copy and run the hook and MCP commands exactly as the manifests
declare them, with ${CLAUDE_PLUGIN_ROOT} / ${CLAUDE_PLUGIN_DATA} substituted
the way Claude Code does.
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from mcp import Client, StdioServerParameters

ROOT = Path(__file__).resolve().parents[1]

pytestmark = pytest.mark.skipif(shutil.which("uv") is None, reason="needs uv")


@pytest.fixture
def installed(tmp_path, fake_herdr):
    """A cache-style copy of the plugin with a broken copied .venv, plus its data dir."""
    copy = tmp_path / "cache" / "herdr-inbox" / "herdr-inbox" / "0.1.1"
    files = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split()
    for rel in files:
        (copy / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / rel, copy / rel)
    # what the real copy left behind: a .venv with config and packages but no interpreter
    broken = copy / ".venv"
    (broken / "bin").mkdir(parents=True)
    (broken / "pyvenv.cfg").write_text("home = /nonexistent\nversion_info = 3.12\n")
    data = tmp_path / "data" / "herdr-inbox-herdr-inbox"
    data.mkdir(parents=True)
    return copy, data


def expand(value: str, copy: Path, data: Path) -> str:
    return value.replace("${CLAUDE_PLUGIN_ROOT}", str(copy)).replace("${CLAUDE_PLUGIN_DATA}", str(data))


def plugin_env(copy, data, **extra):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("UV_", "VIRTUAL_ENV"))}
    env.update(CLAUDE_PLUGIN_ROOT=str(copy), CLAUDE_PLUGIN_DATA=str(data), UV_OFFLINE="1", **extra)
    return env


def test_session_start_hook_runs_from_the_copy(installed):
    copy, data = installed
    hooks = json.loads((copy / "hooks/hooks.json").read_text())["hooks"]
    spec = hooks["SessionStart"][0]["hooks"][0]
    argv = [spec["command"], *(expand(a, copy, data) for a in spec["args"])]
    proc = subprocess.run(argv, input=json.dumps({"session_id": "s", "source": "startup"}),
                          capture_output=True, text=True, timeout=120, env=plugin_env(copy, data), cwd=copy)
    assert proc.returncode == 0, proc.stderr
    assert json.loads(proc.stdout)["hookSpecificOutput"]["hookEventName"] == "SessionStart", proc.stderr
    assert not (copy / ".venv" / "bin" / "python").exists()  # the copy was left alone


async def test_mcp_server_runs_from_the_copy(installed):
    copy, data = installed
    server = json.loads((copy / ".claude-plugin/plugin.json").read_text())["mcpServers"]["herdr-inbox"]
    env = plugin_env(copy, data)
    env.update({k: expand(v, copy, data) for k, v in server.get("env", {}).items()})
    params = StdioServerParameters(command=server["command"], args=[expand(a, copy, data) for a in server["args"]],
                                   env=env, cwd=str(copy))
    async with Client(params, mode="legacy", read_timeout_seconds=120) as client:
        names = {t.name for t in (await client.list_tools()).tools}
    assert "ask_operator" in names


def test_plugin_and_marketplace_versions_agree():
    plugin = json.loads((ROOT / ".claude-plugin/plugin.json").read_text())
    market = json.loads((ROOT / ".claude-plugin/marketplace.json").read_text())
    assert market["plugins"][0]["version"] == plugin["version"]
