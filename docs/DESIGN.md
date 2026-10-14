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
argument carries the agent's own description of itself (e.g. "reviewer PR 42").

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

**Verified delivery (0.5.1).** A successful `herdr agent prompt` only means the
keys were written to the pane. In live use (0.4.0) answers typed while the
agent was mid-turn left no trace in its transcript, yet were recorded as
delivered and seen. So:

- An answer is typed only when `herdr agent get` reports the agent `idle` or
  `done`. Otherwise it is *pending delivery* (`delivery_error = "pending: …"`,
  not a failed attempt, shown as `⏳ Q<n> pending delivery` in the popup) and
  the asking session's MCP retry loop (every 10 s) delivers it at the next idle.
- After typing, the session transcript is tail-read from the size it had
  before the prompt (`transcript.py`, lenient parsing like zoetrope's tailer)
  for a record carrying the message: a `user` record with string content
  (typed while idle), or a `queue-operation` `enqueue` / `attachment`
  `queued_command` (queued mid-turn). Found within 15 s → `delivered_at`,
  `verified_at`, and only then `acknowledged_at`. Not found → `delivery_error =
  "not verified: …"`, retried by the loop, but at most 2 typings in all
  (`MAX_UNVERIFIED_ATTEMPTS`): after that it stays a `⚠` entry in the popup and
  unseen for `check_answers`, so a verifier miss can never type the same answer
  every tick. The verifier accepts string content, list content (its `text`
  blocks joined) and a long prompt that Claude Code stored collapsed — checked on
  a throwaway session 2026-10-14: a ~4000-char prompt is recorded as
  `"\n\n<pasted_content id=\"…\">\n[inbox] …"`, full text inside the wrapper, so
  the wrapper is skipped. No transcript known at all (the
  SessionStart hook records `transcript_path` per session; fallback
  `~/.claude/projects/*/<session>.jsonl`) → sent but left unseen for
  `check_answers`, not retried.
- "Seen" therefore means the agent fetched the answer (`check_answers`,
  `list_my_questions`) or the delivery was verified — never just "sent".
- Recovery, on every `Store()` open (`_reconcile_unverified`): candidates are
  (once, flag in `meta`) every answer from the last 7 days marked seen without
  verification, and (always) rows from the last 24 h with the pre-0.5.1
  signature `acknowledged_at = delivered_at` and no `verified_at` — what a
  0.4/0.5.0 MCP server still running in an older session records when it types
  an answer (it would also type 0.5.1's *pending* rows into a busy pane). Each
  candidate is looked up in its session's transcript (`landed_ids`): found →
  `verified_at`; not found or no transcript → `acknowledged_at` cleared so
  `check_answers` returns it. A fetch by `check_answers` sets `acknowledged_at`
  later than `delivered_at`, so it never matches the signature. Each row is
  reset at most twice (`question_ext.reset_count`), so an old server
  re-acknowledging cannot loop; the reset itself never types anything (the row
  stays delivered). `inbox redeliver <ids>` re-queues answers for delivery.
- Hook latency: the SessionStart hook now opens the store on every session start
  to record the transcript path (schema init, plus the reconcile above). It sits
  inside the never-fail `try`, with SQLite `timeout=2`.
- Probe (2026-10-14, disposable Claude in a tab created and closed for the
  test): typing into a *working* pane did land both times — as a
  `queue-operation enqueue` during plain generation and as an `attachment
  queued_command` during a Bash tool call. The 12:53 loss therefore came from a
  UI state herdr reports as `working` that swallows input (a dialog, an agent
  view, a transcript viewer…), which herdr cannot tell apart. Hence idle-only
  typing plus verification rather than trusting either signal alone.


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
  plugin's `[[startup]]` runs `inbox sync-tokens`.
- **herdr-radar sidebar (0.5.2, on demand).** herdr-radar owns the agent rows
  (a marker-fenced block in herdr's `config.toml`) and has no slot for other
  plugins' tokens; the long-term path is radar's proposed `extra_tokens`
  setting ([hhdebb/herdr-radar#57](https://github.com/hhdebb/herdr-radar/issues/57)).
  Until then `inbox radar-sidebar` / the `radar-sidebar` action (`radar.py`)
  adds `{ token = "$inbox_q", fg = <the row's $title_blocked fg>, bold = true }`
  after `$title_unknown` in every agent row holding radar's `$title_*` tokens
  (`rows` and every `rows_by_agent` entry). Only when the operator runs it (the
  operator chose this over automatic edits): never from `sync-tokens`, the
  startup command or the popup. Safety: edits only between radar's sidebar
  markers (everything else byte for byte: raw bytes in and out, CRLF kept);
  the file must parse before and after; refuses a block holding `[[…]]` tables
  or multi-line strings (not walked); never a row past herdr's 16-token limit
  (radar#40 — one bad row makes herdr reject the whole file), counting every
  top-level cell (inline tables, bare `"$x"`, bare built-ins like
  `"state_icon"`); atomic write (temp file + `os.replace`, into a symlink's
  target so the link stays) with the original bytes as `config.toml.inbox-bak`,
  skipped if the file changed while editing; `herdr server reload-config` only
  after a change; steps aside once radar's own plugin config has an
  `extra_tokens` entry naming `$inbox_q`; `--remove` takes our cells out and is
  never blocked by that. Radar rewrites the block on its configure action; the popup does
  a read-only check in a worker and shows one hint line. Test fixtures are
  radar's own output (`tests/fixtures/radar/generate.js` runs its
  `sidebarBlock` for light/dark, glyph variants and `rows_by_agent` vendors).
- **Tab label prefix** `❓N ` (herdr has no per-tab tokens): on every change the
  pane's current tab (`pane list`) is re-read with `tab get`; the base label is
  the current label with our marks stripped, repeatedly (never stored, so an
  operator rename in the meantime is adopted), and `tab rename` runs only when
  the label differs from `❓<open count in the tab> <base>` (or `<base>` at 0).
  - Each tab the inbox decorates is recorded with every mark applied to it
    (`tab_decor`, cleared once the tab is restored); only those marks, plus the
    default `❓`, are stripped, and repeatedly. Changing the mark can never
    stack prefixes, and an operator label such as `Q4 roadmap` on a tab we
    never decorated is never touched.
  - The setting has one source that herdr-side and Claude-spawned processes
    both read: `tab_prefix` / `prefix_unnamed_tabs` in `config.toml` in the
    plugin config dir (`$HERDR_PLUGIN_CONFIG_DIR`, else
    `herdr plugin config-dir leavemyyard.herdr-inbox`), re-read on each use.
  - Count → `tab get` → `tab rename` runs under a cross-process `flock` on
    `inbox.db.markers.lock` (10 s, then the update is skipped and the next
    change or sync fixes it), with the counts read inside it: concurrent
    processes cannot leave a stale count. The SQLite write lock is never held
    across a herdr call, so a slow or wedged herdr cannot block questions,
    drafts or answers. `sync-tokens` takes the lock per tab; pane tokens are
    updated outside any lock (per pane, idempotent). MCP tools retry on "database
    is locked" with backoff (3 s busy timeout per attempt) and then return a
    "busy, call again" result; the popup shows a notice instead of exiting. A
    marker failure (token, tab label, lock) is logged and never fails the ask or
    answer that triggered it, so a retry never re-files a question.
  - Tabs labelled by position (all digits) are skipped by default: a rename
    would freeze the number.
  - `sync-tokens` reconciles every tab (`workspace list` → `tab list
    --workspace`); `sync-tokens --remove-prefixes` strips them all (before
    uninstalling).
  - Known limitation: herdr has no compare-and-set rename, so an operator
    rename landing in the few milliseconds between our `tab get` and
    `tab rename` is overwritten (the base it had is kept).
  - Questions store the base label.
- **Popup** `inbox` (placement `popup`, 85% × 80%) running `inbox tui`, plus an
  `open` action; the operator binds `prefix+i` to it in their config.

`herdr-plugin.toml` lives at the repo root, id `leavemyyard.herdr-inbox`,
`min_herdr_version = "0.9.0"`, platforms linux and macos. Actions and startup run from the
plugin dir as `uv run --project . inbox ...`; the popup command resolves the
project from `$HERDR_PLUGIN_ROOT`, since a pane's cwd is not documented to be
the plugin dir.

## Questions, groups and drafts (v0.2)

- `ask_operator` mirrors AskUserQuestion where it helps: `options` are strings
  or `{label, description}` (≤ 9), `header` is a ≤ 12-char chip,
  `multi_select` allows several picks. `questions` (≤ 4, AskUserQuestion's
  limit) files related questions in one call: each item is deduped on its own
  and gets its own Q id; the new ones share a `group_id` (the first one's id).
  A single question is a group of one; its receipt keeps the old shape.
- Options may carry a `preview` (monospace text shown beside the list).
  `questions.options` keeps plain label strings (a 0.1.x popup `escape()`s
  them); the full objects are in `question_ext.options_json`.
- When it is the only open question, an answer is sent at once. Otherwise it is
  a draft (`draft_json`: choices, notes per choice, free text) until Submit on
  the popup's Submit tab, which answers only drafted questions:
  `answer` is the one-line text (`Postgres (note: …)`, picks joined by `, `),
  `answer_json` the structure. `inbox answer` is draft + submit in one step.
- Delivery is batched per pane: one `[inbox]` message per pane with all its
  submitted answers, subject to the same session check, retry focus/blocked
  guard and retry cap as before. A batch over herdr's 4000-char argument cap is
  split into several messages, never truncated; a single answer too long for
  any message is sent as a reference ("call check_answers") and stays
  unacknowledged. Picks containing `,`, `"`, parentheses or `·` are quoted.
- The retry also picks up answered rows that were never attempted (the
  answering process died in between) once they are 60 s old.
- **Upgrade safety**: the 0.2 fields (`header`, `multi_select`, `answer_json`,
  `group_id`, drafts) live in a side table `question_ext`, read through the
  `questions_v<N>` view (versioned; older views are never dropped, because processes of the previous release keep querying theirs until their sessions restart). The `questions` table keeps exactly the 0.1.x columns,
  because 0.1.x builds its `Question` from `SELECT *`: sessions still running
  0.1.x keep working against a database 0.2 has opened. `from_row` now ignores
  columns it does not know, so later versions can add columns safely.

## TUI

A **group** is one `ask_operator` call (1–4 questions). The popup works per
group: it lists groups, and inside a group shows that group's questions as
tabs with its own Submit tab.

- **Opening.** Exactly one open group (and no ⚠ entry): straight into that
  group; Esc (or Cancel on its Submit tab) closes the popup. Otherwise the
  **group list**; nothing open: "No open questions" (any key closes; arrivals
  are announced as "N new question groups — press Tab/→ to view").
- **Group names (0.5).** `ask_operator(title=…)` names the whole call (≤60
  chars, sanitised like everything else; strongly requested when asking several
  questions). Stored once per group in the side table `question_groups`
  (joined into the read view as `group_title`; `questions` keeps the 0.1.x
  column set). Older read views stay in place for 0.4 processes. Untitled calls (older sessions, single questions) fall back to
  the first question's header, else its first line (`service.group_name`).
- **Group list**, under a title row ("Open questions · 3 groups · 1 not
  delivered"), rendered like Claude Code's option list (multiline rows):
  `❯ 1. Worker rollout settings · 4 questions · 1 drafted · 4m`, with a dim
  second line `<tab label> · <asked_by> · <header chips>`. Inside a group the
  name sits above the tabs (a lone untitled question shows its header chip
  instead). ⚠ undelivered
  answers are their own entries (`⚠ Q12 not delivered · <tab> · <error>`,
  question → answer), opening a Retry / Stop-showing view. Order: groups from
  the focused pane first, then oldest first; ⚠ entries after them. ↑/↓/j/k move,
  1–9 jump, Enter opens, `d` dismisses the whole group (confirm with `y`), `g`
  goes to its agent's pane, Esc closes. The focused pane comes from
  `HERDR_PLUGIN_CONTEXT_JSON.focused_pane_id`, then `INBOX_FOCUS_PANE` (set by
  `inbox open`), then `pane list`.
- **Inside a group**: tabs only for that group's open questions (a dim
  `│ <tab label>` first) plus its Submit tab. A lone question sends on Enter —
  only ever after the operator opened it (or the popup opened straight into
  it). Several: answering drafts (persisted) and moves to the next tab; Submit
  ("Review your answers", "You have not answered all questions" when some are
  blank, "Ready to submit your answers?" → Submit answers / Cancel) sends that
  group's answered questions, batched per pane; unanswered ones stay open.
  After a send, back to the list at once (the cursor stays on that group; a
  group sent in full is an inert "sending…" row); the popup closes once the
  last send has finished with nothing left. Esc goes back to the list without
  sending. `d` dismisses the current question (confirm with `y`), `g` goes to
  the agent.
- **Question view** (unchanged): header chip, question in bold, numbered
  options with a `❯` cursor, description dimmed, "Notes:" under an option, then
  "Type something." (Claude Code's Other: the answer in single-select, added to
  the picks in multi-select) and, in single-select, "Chat about this" (focuses
  the asking agent's pane; the question stays open). Multi-select ends with a
  Submit/Next row. Previews sit to the right ("No preview available").
- **Colours follow Claude Code's theme roles** (read from the 2.1.299 bundle):
  `suggestion` for the `❯` pointer, the focused option/row and "Notes:";
  `success` for chosen answers (Claude Code's `figures.tick` ✔ after a picked
  option and on tabs, `[✔]` multi-select boxes, the
  Review list's `→ answer`, "N drafted"); `warning` only for warnings (⚠ not
  delivered, "You have not answered all questions", closed placeholders, the
  dismiss confirmation). Dark and light values are Claude Code's own
  (`CC_COLORS`).
- **No background event changes what the next key does.**
  - List rows: arrivals are appended at the end, so the cursor's target and
    every digit keep naming the same rows; a row whose group ended stays in
    place as an inert "✕ … withdrawn/answered/dismissed" placeholder (Enter,
    digits to it, `g`, `d` do nothing) until the operator moves the cursor.
    The list cursor is a target (an entry id), never an index; if its entry is
    gone when the list reappears it rests on nothing. The list does not wrap
    around (↓ on the last row stays put), so an arrival is never where a
    wrapping key lands, and a row that arrived in the background ignores Enter
    and digits for 1 s (`ROW_ARM_SECONDS`; an arrow may move onto it).
  - Nothing is ever added to an open group. A question that closes while
    shown becomes an inert "Q<n> was closed" placeholder tab, dropped once the
    operator switches tabs; other tabs coming and going never rebuild the
    current view (typed text, picks, notes survive). The Submit tab's cursor is
    a target (`("item", qid)` / `"submit"` / `"cancel"`) with an inert rest.
  - Every view (list, question, Submit, ⚠) ignores Enter and digits for 0.4 s
    after it appears, however it was reached; for 0.5 s after a view change
    the operator did not cause, every key is dropped.
  - The send mode (a lone question sends on Enter) is fixed when the group is
    opened.
  - The 0.3 `+N new` marker is gone: its job — keep arrivals out of the tab
    cycle — is done by the list itself, since arrivals are list rows and never
    tabs.
- Every delivery first claims the row with a compare-and-set
  (`delivery_error = 'sending:<time>:<token>'`, taken over after 60 s if its
  process died) before writing any outcome; outcomes are written only while the
  claim is still ours, a retry that backs off gives the claim back, and a claim
  is refused once the agent fetched the answer with `check_answers`. Stranded
  claims and never-claimed answers show as ⚠ entries after 60 s.
- The highlighted row is always scrolled into view. Sends run in a background
  worker; closing the popup waits for an in-flight send.

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
