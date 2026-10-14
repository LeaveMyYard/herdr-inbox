# herdr-inbox

Non-blocking operator questions for coding agents running in [herdr](https://herdr.dev): an MCP tool to ask once, a herdr popup to answer, delivery back into the asking pane.

Status: proof of concept. See [docs/DESIGN.md](docs/DESIGN.md).

The repo is two plugins at once:

- a **Claude Code plugin** (`.claude-plugin/`): the `ask_operator` MCP server, a SessionStart
  hook that tells every session how to use it, and the `herdr-inbox:operator-questions` skill;
- a **herdr plugin** (`herdr-plugin.toml`): the inbox popup, an `open` action, and a startup
  hook that restores the sidebar counts.

## Install

Needs `uv` on `PATH` (for Claude Code and for the herdr server) and herdr ≥ 0.9.0.

```sh
cd /home/leavemyyard/Code/herdr-inbox
uv sync
```

### Claude Code (recommended: plugin)

```sh
claude plugin marketplace add /home/leavemyyard/Code/herdr-inbox
claude plugin install herdr-inbox@herdr-inbox
```

One install registers the MCP server (tools appear as
`mcp__plugin_herdr-inbox_herdr-inbox__ask_operator` etc.) and the hooks. New sessions get a
short `<herdr-inbox>` block in their context; after `/compact` or `--resume` it also lists the
session's questions that are still pending. Outside herdr the hooks stay silent and
`ask_operator` tells the agent to ask in chat. `claude plugin validate /home/leavemyyard/Code/herdr-inbox`
checks the manifests.

Alternative, MCP server only (no hooks; add [docs/agent-instructions.md](docs/agent-instructions.md)
to your `CLAUDE.md` instead):

```sh
claude mcp add --scope user herdr-inbox -- uv run --project /home/leavemyyard/Code/herdr-inbox inbox mcp
```

Use one route, not both, or every session sees two copies of each tool.

### herdr

```sh
herdr plugin link /home/leavemyyard/Code/herdr-inbox
```

Bind `prefix+i` to the popup in `~/.config/herdr/config.toml`, then `herdr server reload-config`:

```toml
[[keys.command]]
key = "prefix+i"
type = "plugin_action"
command = "leavemyyard.herdr-inbox.open"
description = "question inbox"
```

### Sidebar count (`$inbox_q`)

Every pane with open questions carries the pane token `inbox_q = "?N"` (metadata source
`herdr-inbox`); it is cleared when the last one closes. Add `$inbox_q` to a row of the agent
sidebar, for example:

```toml
[ui.sidebar.agents]
rows = [["state_icon", "tab", { token = "$inbox_q", fg = "#c04a4a", bold = true }], ["agent"]]
```

With herdr-radar, the `[ui.sidebar.agents]` rows and `rows_by_agent.claude` live in radar's
managed block: append `{ token = "$inbox_q", fg = "#c04a4a", bold = true, dim = false }` to the
row that holds `$title_working` / `$title_done`, in both places. Radar's `configure` action
rewrites that block, so re-add it after running configure.

## Use

- Agents call `ask_operator(question, options?, context?, asked_by?)` and keep working.
- `prefix+i` opens the inbox: `↑/↓` select, `1`–`9` put an option in the answer box,
  `a`/`Enter` free-text answer (`Enter` sends, `Esc` cancels), `d` dismiss, `g` jump to the
  asking pane, `h` answered/closed history, `q`/`Esc` close.
- The answer is typed into the asking pane as `[inbox] Q12 answered — <question>: <answer>`,
  but only while that pane still holds the conversation that asked. If delivery fails (agent at
  a dialog, pane gone, conversation changed), the TUI shows `answered !`; the asking session's
  MCP server retries every 30 s, and the agent can always get it through `check_answers`.

From a shell:

```sh
uv run inbox list [--all]
uv run inbox answer Q12 "squash it"
uv run inbox dismiss 12
uv run inbox sync-tokens
```

The database lives at `${XDG_STATE_HOME:-~/.local/state}/herdr-inbox/inbox.db`
(`HERDR_INBOX_DB` overrides it).

## Development

```sh
uv run pytest
```

Tests never touch a real herdr: they point `HERDR_BIN_PATH` at a fake that records its argv
and `HERDR_INBOX_DB` at a temp file.
