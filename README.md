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

Claude Code installs the plugin as a **copy** under
`~/.claude/plugins/cache/herdr-inbox/herdr-inbox/<version>/` (git-ignored files included, but a
copied `.venv` loses its interpreter symlinks). The plugin therefore never uses a `.venv` inside
its own directory: the hooks and the MCP server run with
`UV_PROJECT_ENVIRONMENT=${CLAUDE_PLUGIN_DATA}/venv`, i.e.
`~/.claude/plugins/data/herdr-inbox-herdr-inbox/venv`, which uv creates on first use (about
0.5 s, offline from a warm uv cache) and which survives plugin updates.

To pick up new code, bump `version` in `.claude-plugin/plugin.json` **and**
`.claude-plugin/marketplace.json` (the cache directory is keyed by it; an unchanged version means
`update` does nothing), commit, then:

```sh
claude plugin marketplace update herdr-inbox
claude plugin update herdr-inbox@herdr-inbox
```

and restart the Claude sessions (or `/reload-plugins` in each). The previous cache directory is
removed by Claude Code 14 days later.

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

### Tab label count (`❓2 Farm Issues`)

herdr has no per-tab tokens, so while a tab has open questions its label gets a `❓N ` prefix
(`herdr tab rename`), updated as the count changes and removed at 0. Your own label is never
stored: it is recovered by stripping the prefix from the current label each time, so renaming
a prefixed tab is respected. Tabs without a custom name (herdr shows their position, e.g.
`6`) are left alone, because a rename cannot give them back automatic naming. `inbox
sync-tokens` (the plugin's startup hook) reconciles all tabs after a restart or crash.

Configure in `$(herdr plugin config-dir leavemyyard.herdr-inbox)/config.toml`:

```toml
tab_prefix = "❓"            # "" turns the tab prefix off
prefix_unnamed_tabs = false  # true: also prefix tabs named by position (they keep the number)
```

Both the popup and every Claude session read this one file, and edits apply without a
restart. Changing the mark is safe: every mark ever applied is stripped.

## Use

- Agents call `ask_operator(question, options?, header?, multi_select?, context?, asked_by?)`,
  or `ask_operator(questions=[...])` for up to 4 related questions, and keep working.
- `prefix+i` opens the inbox straight into a tabbed view modelled on Claude Code's own question
  prompt. Every open question is a tab: first those from the pane you were in, then the rest
  oldest first, with questions an agent filed together kept adjacent; a dim `│ <tab label>`
  marks where the next agent's questions start, and the tab row scrolls when it overflows. New
  questions are added at the end while it is open. With no open questions it shows
  "No open questions" and closes on any key.
- `↑/↓` or `j/k` move the `❯`, `1`–`9` jump, `n` adds notes to an option, **Type something.**
  is a free answer (in multi-select it is added to the picks), **Chat about this** jumps to the
  asking agent's pane and leaves the question open, `g` goes to the asking agent's pane for any
  question, `d` dismisses the current question (after a one-key confirmation), `Esc` closes
  without sending. Options with a `preview` show it beside the list.
  - Answers that could not be delivered appear as `⚠ Q12 not delivered` tabs with **Retry
    delivery** / **Stop showing it here** (the agent can still fetch it with `check_answers`).
  - If the question on screen is closed meanwhile (withdrawn, answered elsewhere), its tab
    turns into an inert "Q12 was closed" placeholder that ignores Enter and number keys; it
    leaves only when you switch tabs. Other tabs coming and going never touch what you are
    typing.
  - **One open question in total** (decided when the popup opens, and again only when you switch
    tabs, never under your fingers): `Enter` on an option (or text in Type something.) answers
    and sends at once, and the popup closes. Multi-select: `Enter`/`Space` toggle, **Submit**
    sends.
  - **Several**: answering one drafts it and moves to the next tab (`←/→`, `Tab`/`Shift-Tab`
    switch). The last tab is **Submit**: "Review your answers", "You have not answered all
    questions" when some are blank, "Ready to submit your answers?" (**Submit answers** /
    **Cancel**). Submitting sends every answered question at once, one message per pane; the
    unanswered ones stay open. Drafts are kept in the database, so closing the popup loses
    nothing.
- Submitted answers are typed into each asking pane as one line per pane:
  `[inbox] Q12 answered — <question>: Postgres (note: keep the old schema)`, or for several
  `[inbox] answered — Q12 <q>: A · Q13 <q>: B`; only while that pane still holds the
  conversation that asked. If delivery fails (agent at a dialog, pane gone, conversation
  changed), the popup says so; the asking session's MCP server retries every 30 s, and
  the agent can always get it through `check_answers`.

From a shell:

```sh
uv run inbox list [--all]
uv run inbox answer Q12 "squash it"
uv run inbox dismiss 12
uv run inbox sync-tokens
```

The database lives at `${XDG_STATE_HOME:-~/.local/state}/herdr-inbox/inbox.db`
(`HERDR_INBOX_DB` overrides it).

## Uninstall

Remove the tab prefixes first (pane tokens vanish with herdr's next restart):

```sh
uv run --project /home/leavemyyard/Code/herdr-inbox inbox sync-tokens --remove-prefixes
herdr plugin unlink leavemyyard.herdr-inbox
claude plugin uninstall herdr-inbox@herdr-inbox
claude plugin marketplace remove herdr-inbox
```

## Development

```sh
uv run pytest
```

Tests never touch a real herdr: they point `HERDR_BIN_PATH` at a fake that records its argv
and `HERDR_INBOX_DB` at a temp file.
