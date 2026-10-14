# herdr-inbox — design (PoC)

## Problem

Coding agents (Claude Code) running in parallel herdr panes ask the operator
questions in chat, keep working, and then repeat the same question every few
turns because nothing tells them it was received. Blocking dialogs
(`AskUserQuestion`) stop the work, which is also not wanted.

Goal: an agent files a question **once**, gets a receipt, keeps working; the
operator sees every open question from every pane in one place, answers it, and
the answer is delivered back into the right agent's conversation.

## Components

```
Claude Code (pane w6:p32) ──stdio──> MCP server  ──┐
Claude Code (pane w6:p34) ──stdio──> MCP server  ──┼──> SQLite inbox.db <── inbox TUI (herdr popup, prefix+i)
                                                   │          │                  │
                                                   └── herdr CLI: pane tokens,    └── herdr agent prompt <pane>
                                                       notifications                  (answer delivery)
```

One Python package, `herdr_inbox`, one CLI entry point `inbox`:

| Command | Purpose |
|---|---|
| `inbox mcp` | MCP stdio server (registered in Claude Code) |
| `inbox tui` | Textual UI, opened as a herdr popup |
| `inbox list [--all]` | print questions |
| `inbox answer <id> <text>` | answer + deliver (scriptable) |
| `inbox dismiss <id>` | close without answering |
| `inbox sync-tokens` | recompute sidebar tokens for every pane (herdr startup hook) |
| `inbox open` | open the popup (herdr `open` action) |
| `inbox hook session-start\|subagent-start` | Claude Code hooks (see Agent onboarding) |

Runtime: Python ≥ 3.11, `uv`; dependencies `mcp` (FastMCP, called `MCPServer` since mcp 2.x) and `textual`.
Storage: stdlib `sqlite3`, WAL mode, at
`${XDG_STATE_HOME:-~/.local/state}/herdr-inbox/inbox.db` (overridable with
`HERDR_INBOX_DB` for tests). Multiple MCP server processes and the TUI share it.

## Identity — who asked

The MCP server is spawned by the Claude process, so it inherits the pane's herdr
environment: `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH`.
On every `ask_operator` call it resolves, via the `herdr` CLI
(`HERDR_BIN_PATH` or `herdr` on PATH):

- `herdr pane get <pane>` → `tab_id`, `agent_session.value` (Claude session id)
- `herdr tab get <tab>` → tab `label` (the name the operator gave the tab)

Resolve per call, not once at startup: `/clear` changes the session id, and a
pane can be moved. Session fallback when herdr gives none:
`CLAUDE_CODE_SESSION_ID` from the environment.

A `claude` nested inside the pane's agent (e.g. `claude -p` run from its Bash
tool) inherits `HERDR_PANE_ID` but is not that agent. It is recognised from the
process tree: walking up from the server or hook, the nearest `claude` is our
own; a second `claude` further up means nested (verified on a real nested run).
An unreadable tree counts as not nested. A nested session is treated like one
outside herdr. (Comparing `CLAUDE_CODE_SESSION_ID` with the pane's session was
rejected: the MCP server's environment is fixed at spawn, so it would misfire
after `/clear`, and the SessionStart hook races herdr's own session report.)

Outside herdr (no `HERDR_PANE_ID`) or nested, nobody watches the inbox, so
nothing is filed: `ask_operator` returns `{"status": "unavailable"}` telling the agent to
ask in chat, the hooks print nothing, and no herdr call is made. The plugin is
installed at user scope, so this covers plain terminals, IDEs and `claude -p`.

Which **subagent** asked is out of scope for the PoC; the optional `asked_by`
argument carries the agent's own description of itself (e.g. "reviewer PR 595").

## Data model

`questions`:
`id` (int, shown as `Q<id>`), `created_at`, `pane_id`, `tab_id`, `tab_label`,
`session_id`, `cwd`, `asked_by`, `question`, `options` (JSON list or null),
`context`, `norm` (normalized question text), `status`
(`open` | `answered` | `dismissed` | `withdrawn`), `answer`, `answered_at`,
`delivered_at`, `delivery_error`, `acknowledged_at`, `repeat_count`.

## MCP tools

All tools are non-blocking and return immediately.

### `ask_operator(question, options?, context?, asked_by?)`
- Dedupe: normalize (lowercase, collapse whitespace, strip punctuation) and look
  for a match from the same `session_id` (fallback: same `pane_id`):
  - open match → increment `repeat_count`, return
    `{"id","status":"already_open"}` with the message "Q12 is already with the
    operator. Do not ask again; keep working."
  - answered match (answered within the last hour) → return the stored answer
    right away. Older answers are not reused: the same words later are
    probably a new instance ("Proceed with the next step?").
  - The lookup and the insert run in one `BEGIN IMMEDIATE` transaction, so
    parallel calls in one turn or two subagents cannot file the same Q twice.
- Otherwise insert, notify, update tokens, return
  `{"id": 12, "status": "open"}` with the message:
  "Recorded as Q12. The operator will answer asynchronously; the answer will
  arrive in this conversation as a message starting with `[inbox] Q12`.
  Continue with work that does not depend on it. Do not repeat the question in
  chat or call ask_operator for it again."
- Tool description must say the same thing, plus: use this instead of asking in
  chat whenever work can continue; keep the question self-contained (the
  operator reads it without the chat history); put choices in `options`.

### `check_answers()`
Answered questions for the current session not yet acknowledged; marks them
acknowledged. Fallback for missed deliveries and after compaction.

### `list_my_questions(include_closed=false)`
The current session's questions with status. Lets an agent recover after
compaction instead of re-asking.

### `withdraw_question(id, reason)`
The agent resolved it itself; status `withdrawn`, removed from the operator's view.

## Answer delivery

On answer (TUI or `inbox answer`): set status, then if the question has a pane
and the pane's live agent is **the conversation that asked**
(`herdr agent get` → `agent_session.value == session_id`; any live agent when
the question has no session id), run
`herdr agent prompt <pane_id> "[inbox] Q12 answered — <question, truncated to 80 chars>: <answer>"`
without `--wait`. Claude Code queues a prompt submitted while it is working.
After `/clear`, a restart or another agent in the pane the answer is not typed
anywhere (`delivery_error = "conversation changed …"`).

A successful delivery sets `delivered_at` and `acknowledged_at` (typed into the
conversation counts as seen). A failure records `delivery_error`; the answer
stays available through `check_answers` and the compact/resume hook, and the
asking session's own MCP server retries it every 30 s (it is the one process
that lives exactly as long as the conversation). A retry is skipped while the
pane is focused or the agent is `blocked` (the operator may be typing there or
answering the dialog; the prompt would merge with their input). "No live agent"
and "conversation changed" are given up after 10 attempts. The TUI shows
delivery state.

The first delivery, from the popup, has no focus guard: the popup is modal and
usually opened over the very pane whose agent asked, so the operator is typing
in the popup, not the pane, and guarding would turn the main path into a 30 s
wait. A blocked agent is refused by `agent prompt` itself (`agent_blocked`).

PoC limitation: an `agent prompt` that times out after herdr already submitted
it is recorded as failed and retried, so the agent can receive that answer twice.

Everything passed to herdr goes through one sanitizer: line breaks and tabs
become spaces, all other C0/C1 control characters and DEL are dropped, and each
argument is capped at 4000 characters, so agent-written text cannot smuggle
escape or key sequences into a prompt, notification or token.

Known PoC risk: the prompt is typed into the pane, so it would merge with
anything the operator is half-way through typing in that pane.

## herdr surface

- **Notification** on each new question:
  `herdr notification show "Q12 · <tab label>" --body "<question>" --sound request`.
- **Pane token** `inbox_q` = `"?N"` (N open questions) or cleared, via
  `herdr pane report-metadata <pane> --source herdr-inbox --token inbox_q=...`,
  updated on every state change. Tokens do not survive a herdr restart, so the
  plugin's `[[startup]]` runs `inbox sync-tokens`. herdr-radar owns the sidebar
  row layout; the README documents how to add `$inbox_q` to it (the PoC does
  not edit the operator's herdr config).
- **Popup** `inbox` (placement `popup`, 85% × 80%) running `inbox tui`, plus an
  `open` action; the operator binds `prefix+i` to it in their config.

`herdr-plugin.toml` lives at the repo root, id `leavemyyard.herdr-inbox`,
`min_herdr_version = "0.9.0"`, platforms linux and macos. Actions and startup run from the
plugin dir as `uv run --project . inbox ...`; the popup command resolves the
project from `$HERDR_PLUGIN_ROOT`, since a pane's cwd is not documented to be
the plugin dir.

## TUI

- List of open questions, newest first: `Q<id>`, tab label, `asked_by`, age,
  repeat count, first line of the question.
- Detail: full question, context, options. Number keys pick an option; `a` or
  Enter opens a free-text answer box; Enter sends.
- `d` dismiss · `g` focus the asking agent's pane (`herdr agent focus`) ·
  `h` toggle answered/closed history · `q`/Esc close the popup.
- Polls the database every 2 s.

## Agent onboarding

Every Claude Code session learns the inbox by default, without a hand-edited
CLAUDE.md, the way the superpowers plugin does it: the repo is also a Claude
Code plugin.

- `.claude-plugin/plugin.json` declares the plugin `herdr-inbox` and bundles
  the MCP server inline (`uv run --project ${CLAUDE_PLUGIN_ROOT} inbox mcp`).
  Inline rather than a root `.mcp.json`, because a root `.mcp.json` is also
  Claude Code's project-scope MCP file for anyone working in this repo.
- Claude Code installs the plugin as a copy in
  `~/.claude/plugins/cache/herdr-inbox/herdr-inbox/<version>/`, git-ignored
  files included, and a copied `.venv` has no interpreter. Every plugin command
  therefore runs `uv run --frozen --no-dev --project ${CLAUDE_PLUGIN_ROOT}` with
  `UV_PROJECT_ENVIRONMENT=${CLAUDE_PLUGIN_DATA}/venv` (MCP: the server's `env`;
  hooks: exec-form `env UV_PROJECT_ENVIRONMENT=… uv …`), so the environment
  lives in the plugin's persistent data dir and the copy is only read.
  `tests/test_plugin_copy.py` builds such a copy (broken `.venv` included) and
  runs the commands exactly as the manifests declare them.
- `.claude-plugin/marketplace.json` is a local single-plugin marketplace
  (`source: "./"`): `claude plugin marketplace add <repo>` +
  `claude plugin install herdr-inbox@herdr-inbox`.
- `hooks/hooks.json`:
  - `SessionStart`, matcher `startup|resume|clear|compact|fork`, runs
    `inbox hook session-start`, which prints
    `{"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": …}}`.
    The text is short (under 250 words; it lands in every session): use
    `ask_operator` for anything you would otherwise ask in chat while you can
    keep working; ask once; never restate it; answers arrive as
    `[inbox] Q<n> answered`; after compaction/resume call `list_my_questions`
    first; blocking/destructive confirmations keep the normal flow; pass the
    rule on in subagent briefs; details in the skill. On `compact` and
    `resume` it appends the session's pending questions (`Q12 (open): …`, and
    answered-but-unseen ones), matched by session id only: `--resume` keeps
    the session id (checked with a real `claude -p --resume` run).
  - `SubagentStart` runs `inbox hook subagent-start`: one paragraph for
    subagents (ask once with `asked_by`, do not wait, the answer goes to the
    main session). The hooks reference documents `additionalContext` for
    SubagentStart. It does not say SessionStart fires for subagents, and the
    sub-agents page says a subagent starts with a fresh context (own prompt,
    CLAUDE.md, git status), so SessionStart context is not assumed to reach
    them; the main agent is also told to pass the rule on.
  - Both are fast (no mcp/textual import; ~40 ms through `uv run`) and never
    fail the session: any error means exit 0 with no output. Outside herdr
    they print nothing.
- `skills/operator-questions/SKILL.md` holds the fuller guidance (phrasing a
  self-contained question, options, `asked_by`, withdrawing); the hook text
  points to it as `herdr-inbox:operator-questions`.

`docs/agent-instructions.md` keeps the equivalent CLAUDE.md snippet for the
manual route (`claude mcp add` without the plugin).

## Out of scope for the PoC

Subagent attribution (planned: a `PreToolUse` hook on `ask_operator` that reads
the hook payload's `agent_id`), zoetrope node highlighting, blocking questions,
remote/mobile answering, multi-machine sync.
