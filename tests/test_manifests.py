"""Manifest sanity checks that run anywhere (no claude, no herdr needed)."""

import json
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def _json(rel):
    return json.loads((ROOT / rel).read_text())


def _toml(rel):
    return tomllib.loads((ROOT / rel).read_text())


def test_versions_match_everywhere():
    plugin = _json(".claude-plugin/plugin.json")["version"]
    market = _json(".claude-plugin/marketplace.json")["plugins"][0]["version"]
    herdr = _toml("herdr-plugin.toml")["version"]
    project = _toml("pyproject.toml")["project"]["version"]
    assert plugin == market == herdr == project


def test_claude_plugin_required_fields():
    plugin = _json(".claude-plugin/plugin.json")
    for key in ("name", "version", "description", "license", "mcpServers"):
        assert plugin.get(key), key
    market = _json(".claude-plugin/marketplace.json")
    assert market["name"] and market["owner"]["name"]
    entry = market["plugins"][0]
    assert entry["name"] == plugin["name"]
    assert entry["source"] == "./"


def test_hooks_manifest_parses():
    hooks = _json("hooks/hooks.json")["hooks"]
    assert {"SessionStart", "SubagentStart"} <= set(hooks)


def test_herdr_manifest_required_fields():
    m = _toml("herdr-plugin.toml")
    for key in ("id", "name", "version", "min_herdr_version"):
        assert m.get(key), key
    assert [p["id"] for p in m["panes"]] == ["inbox"]
    assert {a["id"] for a in m["actions"]} == {"open", "sync-tokens", "radar-sidebar"}
    # GitHub installs run [[build]] before the first popup.
    assert m["build"][0]["command"][:2] == ["uv", "sync"]
    assert m["build"][1]["command"] == ["sh", "scripts/install-bin.sh"]  # the Rust binary (plan §4 item 3)


def test_license_declared_and_present():
    assert _toml("pyproject.toml")["project"]["license"] == "MIT"
    assert (ROOT / "LICENSE").read_text().startswith("MIT License")


def test_ui_mod_is_its_own_plugin():
    """herdr-inbox-ui (early-access mods API) ships as a separate marketplace entry, so the
    core plugin never depends on it."""
    ui = _json("plugins/herdr-inbox-ui/.claude-plugin/plugin.json")
    entries = {p["name"]: p for p in _json(".claude-plugin/marketplace.json")["plugins"]}
    entry = entries["herdr-inbox-ui"]
    assert entry["source"] == "./plugins/herdr-inbox-ui"
    assert entry["version"] == ui["version"] and ui["name"] == "herdr-inbox-ui"
    assert _json("plugins/herdr-inbox-ui/hooks/hooks.json") == {"modules": ["./register.tsx"]}
    assert "modules" not in _json("hooks/hooks.json")
