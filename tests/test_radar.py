"""On-demand $inbox_q in herdr-radar's sidebar rows (radar.py).

The fixtures are radar's own output: tests/fixtures/radar/generate.js runs radar's
lib/managed-config.js sidebarBlock for dark/light, font/text/no glyphs, with and
without rows_by_agent vendors."""

import re
import tomllib
from pathlib import Path

import pytest

from herdr_inbox import cli, radar, service

FIXTURES = sorted((Path(__file__).parent / "fixtures" / "radar").glob("*.block.toml"))
BEFORE = '# my herdr config\n[keys]\nprefix = "ctrl-b"\n\n[ui]\nsidebar_width = 40   # keep "this" [as is]\n\n'
AFTER = "\n# trailing comment, kept\n"


@pytest.fixture
def herdr_config(fake_herdr, monkeypatch):
    path = Path(radar.config_path())
    path.parent.mkdir(parents=True, exist_ok=True)
    monkeypatch.setenv("FAKE_HERDR_PLUGINS", "leavemyyard.herdr-inbox,hhdebb.herdr-radar")
    return path


def write(path, block):
    text = BEFORE + block + AFTER
    path.write_text(text, encoding="utf-8")
    return text


def agent_rows(text):
    """(table, key, row) for every agent row, via the module's own walker."""
    block = text[text.index(radar.START):text.index(radar.END)]
    data = tomllib.loads(text)["ui"]["sidebar"]["agents"]
    rows = [("rows", r) for r in data["rows"]] + [(k, r) for k, v in data.get("rows_by_agent", {}).items() for r in v]
    return block, rows


def title_rows(rows):
    return [(k, r) for k, r in rows if any(isinstance(c, dict) and c.get("token", "").startswith("$title_") for c in r)]


@pytest.mark.parametrize("fixture", FIXTURES, ids=lambda p: p.name.split(".")[0])
def test_adds_the_token_to_every_agent_title_row_and_nothing_else(herdr_config, fake_herdr, fixture):
    original = write(herdr_config, fixture.read_text(encoding="utf-8"))
    assert radar.ensure() == "changed"
    text = herdr_config.read_text(encoding="utf-8")
    # outside radar's markers: byte for byte
    assert text.startswith(BEFORE) and text.endswith(radar.END + "\n" + AFTER)
    # the spaces table and everything that is not a title row: unchanged
    new, old = tomllib.loads(text), tomllib.loads(original)
    assert new["ui"]["sidebar"]["spaces"] == old["ui"]["sidebar"]["spaces"]
    _, rows = agent_rows(text)
    titled = title_rows(rows)
    assert titled and len(titled) == 1 + len(old["ui"]["sidebar"]["agents"].get("rows_by_agent", {}))
    for _, row in titled:
        tokens = [c["token"] if isinstance(c, dict) else c for c in row]
        assert tokens[tokens.index("$title_unknown") + 1] == "$inbox_q"
        assert tokens.count("$inbox_q") == 1 and len(tokens) <= radar.ROW_TOKEN_LIMIT
        blocked = next(c for c in row if isinstance(c, dict) and c.get("token") == "$title_blocked")
        ours = next(c for c in row if isinstance(c, dict) and c.get("token") == "$inbox_q")
        assert ours == {"token": "$inbox_q", "fg": blocked["fg"], "bold": True, "dim": False}
    assert text.count("$inbox_q") == len(titled)
    # atomic write with a backup; herdr reloaded once
    assert (herdr_config.parent / "config.toml.inbox-bak").read_text(encoding="utf-8") == original
    assert len(fake_herdr.calls_of("server", "reload-config")) == 1
    # idempotent, and --remove restores the original byte for byte
    fake_herdr.reset()
    assert radar.ensure() == "unchanged" and herdr_config.read_text(encoding="utf-8") == text
    assert radar.ensure(remove=True) == "changed"
    assert herdr_config.read_text(encoding="utf-8") == original
    assert radar.ensure(remove=True) == "unchanged"
    assert len(fake_herdr.calls_of("server", "reload-config")) == 1


def test_matches_the_operators_hand_patch(herdr_config):
    """The operator's manual edit (coordinator, 2026-10-14): `$inbox_q` right after
    `$title_unknown`, fg #c04a4a (radar's dark blocked red), bold, not dim."""
    write(herdr_config, (FIXTURES[0].parent / "dark-font.block.toml").read_text(encoding="utf-8"))
    radar.ensure()
    assert '{ token = "$title_unknown", fg = "#907aa9", bold = false, dim = false }, ' \
           '{ token = "$inbox_q", fg = "#c04a4a", bold = true, dim = false }' in herdr_config.read_text(encoding="utf-8")


def test_no_radar_no_block_or_invalid_toml_are_left_alone(herdr_config, fake_herdr, monkeypatch):
    block = FIXTURES[0].read_text(encoding="utf-8")
    original = write(herdr_config, block)
    monkeypatch.setenv("FAKE_HERDR_PLUGINS", "leavemyyard.herdr-inbox")
    assert radar.ensure() == "no-radar"
    monkeypatch.setenv("FAKE_HERDR_PLUGINS", "hhdebb.herdr-radar")
    herdr_config.write_text(original.replace(radar.START, "# radar block was here"))
    assert radar.ensure() == "no-block"
    broken = original.replace("[keys]", "[keys")
    herdr_config.write_text(broken)
    assert radar.ensure() == "invalid" and herdr_config.read_text() == broken
    assert radar.edited(broken, remove=False) is None
    assert not fake_herdr.calls_of("server", "reload-config")
    assert not (herdr_config.parent / "config.toml.inbox-bak").exists()


def test_never_crosses_herdrs_16_token_row_limit(herdr_config):
    block = (FIXTURES[0].parent / "dark-no-vendors.block.toml").read_text(encoding="utf-8")
    extra = "".join(f', {{ token = "$x{i}", fg = "#000000", bold = false, dim = false }}' for i in range(5))
    full = re.sub(r'(\{ token = "\$title_unknown"[^}]*\})', lambda m: m.group(1) + extra, block, count=1)
    assert full != block
    write(herdr_config, full)
    _, rows = agent_rows(herdr_config.read_text())
    assert max(len(r) for _, r in title_rows(rows)) == radar.ROW_TOKEN_LIMIT
    assert radar.ensure() == "unchanged"
    assert "$inbox_q" not in herdr_config.read_text()


def test_steps_aside_once_radar_carries_the_token(herdr_config, fake_herdr, tmp_path, monkeypatch):
    """hhdebb/herdr-radar#57: an `extra_tokens` setting naming $inbox_q in radar's config."""
    original = write(herdr_config, FIXTURES[0].read_text(encoding="utf-8"))
    radar_cfg = tmp_path / "radar-config"
    radar_cfg.mkdir()
    (radar_cfg / "config.toml").write_text('extra_tokens = ["$inbox_q"]\n')
    monkeypatch.setenv("FAKE_HERDR_RADAR_CONFIG_DIR", str(radar_cfg))
    assert radar.ensure() == "stepped-aside"
    assert radar.ensure(remove=True) == "unchanged"  # --remove is never blocked (nothing of ours here)
    assert herdr_config.read_text() == original and not radar.missing()
    (radar_cfg / "config.toml").write_text('extra_tokens = ["$other"]\n')
    assert radar.ensure() == "changed"


def test_a_concurrent_write_wins(herdr_config, fake_herdr, monkeypatch):
    write(herdr_config, FIXTURES[0].read_text(encoding="utf-8"))
    real = radar.edited

    def edited_while_radar_rewrites(text, remove):
        herdr_config.write_text(text + "# radar rewrote its block\n")
        return real(text, remove)

    monkeypatch.setattr(radar, "edited", edited_while_radar_rewrites)
    assert radar.ensure() == "conflict"
    assert herdr_config.read_text().endswith("# radar rewrote its block\n") and "$inbox_q" not in herdr_config.read_text()
    assert not fake_herdr.calls_of("server", "reload-config")


def test_missing_is_read_only(herdr_config, fake_herdr):
    original = write(herdr_config, FIXTURES[0].read_text(encoding="utf-8"))
    assert radar.missing()
    assert herdr_config.read_text() == original and not fake_herdr.calls_of("server", "reload-config")
    radar.ensure()
    assert not radar.missing()
    (Path(radar.tabs._config_dir()) / "config.toml").write_text('tab_prefix = ""\nradar_sidebar_hint = false\n')
    radar.ensure(remove=True)
    assert not radar.missing()


def test_nothing_runs_it_automatically(herdr_config, store, fake_herdr):
    """Operator 2026-10-14: on demand only. sync-tokens (also herdr's startup command) and
    the popup never edit herdr's config."""
    original = write(herdr_config, FIXTURES[0].read_text(encoding="utf-8"))
    service.sync_tokens(store)
    service.sync_tokens(store, remove_prefixes=True)
    assert herdr_config.read_text() == original
    assert not fake_herdr.calls_of("server", "reload-config")


async def test_popup_shows_a_hint_and_edits_nothing(herdr_config, store, fake_herdr):
    from herdr_inbox.tui import InboxApp

    original = write(herdr_config, FIXTURES[0].read_text(encoding="utf-8"))
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert app.has_class("radar-missing")
        assert "radar-sidebar action" in str(app.query_one("#radar-hint").render())
    assert herdr_config.read_text() == original
    assert not fake_herdr.calls_of("server", "reload-config")


async def test_popup_has_no_hint_without_radar(store, fake_herdr):
    from herdr_inbox.tui import InboxApp

    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await app.workers.wait_for_complete()
        await pilot.pause()
        assert not app.has_class("radar-missing")


def test_cli(herdr_config, capsys):
    write(herdr_config, FIXTURES[0].read_text(encoding="utf-8"))
    assert cli.main(["radar-sidebar"]) == 0
    assert "added to herdr-radar's sidebar rows" in capsys.readouterr().out
    assert cli.main(["radar-sidebar", "--remove"]) == 0
    assert "removed from" in capsys.readouterr().out
    herdr_config.unlink()
    assert cli.main(["radar-sidebar"]) == 1


# -- review of #10 -------------------------------------------------------------------------


def test_every_cell_form_counts_toward_the_16_token_limit():
    """H1: bare built-ins ("state_icon", "tab") are cells too; rules = [...] are not."""
    cells = ['"state_icon"', '"tab"', '"$bare"'] + [f'{{ token = "$t{i}" }}' for i in range(11)] + [
        '{ token = "$title_blocked", fg = "#c04a4a", rules = [{ contains = "[", fg = "#000000" }, { equals = "x" }] }',
        '{ token = "$title_unknown" }']
    row = "[" + ", ".join(cells) + "]"
    assert radar.tokens_in(row) == len(cells) == 16
    assert radar._edit_row(row, False) == row
    assert radar.tokens_in("[]") == 0 and radar.tokens_in('["$gap"]') == 1


@pytest.mark.parametrize("newline", ["\n", "\r\n"], ids=["lf", "crlf"])
def test_line_endings_and_backup_are_kept_byte_for_byte(herdr_config, newline):
    """M1: no newline translation anywhere; the backup is the original bytes."""
    original = (BEFORE + FIXTURES[0].read_text(encoding="utf-8") + AFTER).replace("\n", newline).encode()
    herdr_config.write_bytes(original)
    assert radar.ensure() == "changed"
    after = herdr_config.read_bytes()
    assert after.count(b"\r\n") == original.count(b"\r\n") and b"$inbox_q" in after
    assert (herdr_config.parent / "config.toml.inbox-bak").read_bytes() == original
    assert radar.ensure(remove=True) == "changed"
    assert herdr_config.read_bytes() == original


def test_a_symlinked_config_is_edited_through_the_link(herdr_config, tmp_path):
    target = tmp_path / "dotfiles" / "herdr.toml"
    target.parent.mkdir()
    original = BEFORE + FIXTURES[0].read_text(encoding="utf-8") + AFTER
    target.write_text(original)
    herdr_config.symlink_to(target)
    assert radar.ensure() == "changed"
    assert herdr_config.is_symlink() and "$inbox_q" in target.read_text()
    assert radar.ensure(remove=True) == "changed" and target.read_text() == original


def test_step_aside_needs_a_real_extra_tokens_entry_and_never_blocks_remove(herdr_config, tmp_path, monkeypatch):
    original = write(herdr_config, FIXTURES[0].read_text(encoding="utf-8"))
    radar_cfg = tmp_path / "radar-config"
    radar_cfg.mkdir()
    monkeypatch.setenv("FAKE_HERDR_RADAR_CONFIG_DIR", str(radar_cfg))
    assert radar.ensure() == "changed"
    # an empty extra_tokens (in radar's config, or written into its block) is not radar carrying ours
    (radar_cfg / "config.toml").write_text("extra_tokens = []\n# $inbox_q mentioned in a comment\n")
    herdr_config.write_text(herdr_config.read_text().replace("[ui.sidebar.agents]\n", "[ui.sidebar.agents]\nextra_tokens = []\n", 1))
    assert not radar.stepped_aside()
    herdr_config.write_text(herdr_config.read_text().replace("extra_tokens = []\n", "", 1))
    (radar_cfg / "config.toml").write_text('extra_tokens = ["$inbox_q"]\n')
    assert radar.stepped_aside() and radar.ensure() == "stepped-aside"
    assert radar.ensure(remove=True) == "changed" and herdr_config.read_text() == original


@pytest.mark.parametrize("oddity", ['[[ui.sidebar.agents.extra]]\nname = "x"\n', 'note = """multi [\nline ]"""\n'],
                         ids=["array-of-tables", "multiline-string"])
def test_unsupported_toml_inside_the_block_is_refused(herdr_config, oddity):
    block = FIXTURES[0].read_text(encoding="utf-8").replace("[ui.sidebar.spaces]\n", oddity + "[ui.sidebar.spaces]\n")
    original = write(herdr_config, block)
    assert tomllib.loads(original)
    assert radar.ensure() == "unsupported" and herdr_config.read_text() == original
