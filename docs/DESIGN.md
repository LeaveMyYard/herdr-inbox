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

**The delivery worker (0.6.1).**
- **Why:** until 0.6.0 each asking session's own MCP server retried deliveries into its own
  pane. A session started before an update keeps its old server, so answers to long-lived
  sessions could go unretried. Live, 2026-10-15: Q49–Q52 were typed at 06:28:18 while the
  agent's background-agents panel had focus. They never landed, and the session's
  pre-0.5.1 server had already marked them seen. Reconcile un-saw them, but nothing retried
  them until the agent called `check_answers` 11 minutes later.
- **Now** (`worker.py`): one herdr-side worker **per herdr server**, a singleton held by an
  exclusive flock on `worker-<hash of the server's socket>.lock` next to the database.
  - Each question records the server it was asked on (`question_ext.herdr_server`, from
    `HERDR_SOCKET_PATH`). A worker handles only its own server's rows; a send for another
    server's row starts that server's worker instead.
  - Rows from before 0.6.1 carry no server. They are handled only when the pane's live
    conversation is the asking session, never on a pane-id match alone, and burn no retry
    caps otherwise.
  - Spawns strip `UV_*` and `VIRTUAL_ENV`, so a worker started from an MCP server never
    makes uv resync that MCP venv for the herdr checkout.
  - It is started detached (`ensure()`) from herdr's `[[startup]]` (`inbox worker
    --ensure`), the popup, `inbox answer` / `redeliver`, and every 0.6.1+ MCP server.
  - Every `TICK` (5 s) it runs `Store.reconcile()` and then `deliver_batch(retry=True)`
    for **all** panes. Claims serialize it with old servers' loops.
  - Session MCP servers keep only ask/check/list/withdraw.
  - It exits after 10 idle minutes, and the next answer starts it again. "Idle" counts
    only the answers it serves: other servers' rows and unmatched pre-0.6.1 rows never keep
    it alive.
- **An old server's lost delivery is requeued:** reconcile now also sends such an answer
  again. It applies when the row carries the pre-0.5.1 signature (`acknowledged_at =
  delivered_at`) and was delivered between `REQUEUE_SETTLE` (20 s, more than the 15 s verify
  window, since Claude Code writes the record a moment after the old server's DB write) and
  an hour ago (`REQUEUE_WINDOW`). Its transcript must actually have been read and must lack
  it. Without a transcript the row is only un-seen, never retyped blind.
- **Look before typing:** whenever the transcript is known, `deliver_batch` checks it
  before **every** typing attempt (0.6.3; before, only rows typed before). This covers a
  late landing and a sender killed mid-verify whose stale claim is being taken over.
  The check matches each answer's **full expected text** (`transcript.landed_exact`): the
  single message, its batch segment `Q<n> <first line>: <answer>` at a segment start, or
  the long-answer reference. If only the reference landed, the row is verified but **not**
  acknowledged, since the agent was only told to call check_answers. The id alone isn't enough, so an id written inside another
  answer ("… · Q28 can wait") can't make a never-typed answer count as landed. Records
  older than the answer are ignored too, since ids restart if `inbox.db` is recreated.
  The expected strings are what herdr actually types (`herdr.clean`: tabs, newlines and
  U+2028 become spaces), and both sides are compared with runs of whitespace collapsed.
  Reconcile uses the same check. A late record marks it
  verified instead of typing a second copy. The "operator in this pane" check is repeated
  right before each prompt. The row then becomes `pending: requeued`, delivered only at
  idle/done. Older rows are only un-seen, as before. `reset_count` (≤ 2) bounds any
  back-and-forth with an old server.
- **One busy try:** an answer typed while busy that isn't seen (`MAX_BUSY_ATTEMPTS` = 1)
  waits for the next idle/done. The popup shows it at once as `⚠ not delivered — agent
  busy; will retry when it is idle`.
- **Panel focus:** before typing, the worker reads the pane's visible lines (read-only).
  If Claude Code's **footer** reads `↑/↓ to select` (the background-agents panel or a
  subagent view has focus), it waits, and the popup says so. The footer is the first
  non-empty line below the input box's last `───` border, so the phrase in ordinary
  output (a diff, this repo's docs) never counts. Send now overrides the wait; the
  transcript check catches a real swallow.
- **The worker is always there when needed:** every non-retry send (the popup's answer,
  Submit, Send now/Retry, `inbox answer`) calls `worker.ensure()`. A worker that idles
  out re-checks for pending answers after releasing its lock and starts a successor if
  one arrived meanwhile. A worker that finds the installed version changed after a tick
  hands over to the new code (`uv run --project <root> inbox worker`). "Changed" means
  the checkout's `pyproject.toml` version differs from the running code's own package
  version. If no successor can be started (no checkout, no uv), the worker keeps serving
  rather than respawning itself in a loop. The plugin root is cached.
- **Notification:** an answer still unseen 5 min after it was answered, or out of idle
  tries, raises one herdr notification per pane, once per answer (`notified_at`):
  - normally "Q49–Q52 not delivered to <tab>", which says the agent can fetch them
    with check_answers;
  - for an agent waiting at a prompt, "waiting: <tab> is at a prompt (Q…)", which
    points at the pane.

  An answer sent where no transcript could be found (unverifiable, most likely landed)
  doesn't notify.
- **Probe 3** (2026-10-15, a disposable haiku session in a tab created and closed for the
  test). The agent was waiting on 3 background subagents ("✻ Waiting for 3 background
  agents to finish"); herdr said `working`.
  - With the prompt focused, typed text landed at once.
  - With the agents panel focused (↓, footer `↑/↓ to select`), typed text left no record
    and the prompt box stayed empty: the text was swallowed.
  - With a subagent's view selected, the text sat unsent in that view's input.
  - After Esc back to the prompt, typing landed again.
  - ← ("for agents") opens a "Background this session?" dialog.

**Verified delivery (0.5.1).** A successful `herdr agent prompt` only means the
keys were written to the pane. In live use (0.4.0) answers typed while the
agent was mid-turn left no trace in its transcript, yet were recorded as
delivered and seen. So:

- When to type (0.5.4), by `herdr agent get` status:
  - **idle / done:** typed now.
  - **blocked** (a prompt or dialog, which swallows typed text): *pending
    delivery*, retried every 10 s by the asking session's MCP loop, for however
    long it takes. The popup says `waiting: agent is at a prompt/dialog`.
  - **working, or anything else:** pending for `BUSY_GRACE` = 20 s, counted
    from `question_ext.pending_since`. That is the first *busy* moment (time
    spent blocked doesn't count), and it is cleared on delivery. After the grace
    the answer is typed anyway, with the transcript check as the safety net. The
    popup says `agent busy — will send in N s`.
  - Misses while busy (typed, not seen) are counted separately
    (`question_ext.busy_attempts`, at most `MAX_BUSY_ATTEMPTS` = 3) and stay
    pending, not failed. After 3 the answer waits for idle (`agent busy — will
    send when it is idle`). Only misses at idle/done count toward
    `MAX_UNVERIFIED_ATTEMPTS` = 2, so an answer always gets its tries at rest
    before it is parked as ⚠.
  - A transcript record that shows up after the 15 s verify window counts as a
    miss, so the next try types a second copy. The caps bound this (the probe
    saw the queued record immediately), so the agent may at worst get the
    answer twice.

  Pending is never a failed attempt. A pending row in the popup offers **Send
  now** (`1`), which skips the grace and the focused-pane wait, but not
  `blocked`.
- Why not idle-only (0.5.1–0.5.3): herdr reports `working` for as long as
  Claude Code waits on a background workflow ("✻ Waiting for 1 dynamic workflow
  to finish"), which can be hours. Claude Code accepts typed input in that state:
  live Q45, typed by a pre-0.5.1 server during the wait, landed as a `user`
  record and was answered within 3 s. The 0.5.3 servers held it as pending the
  whole time, which the operator read as "fails to be uploaded".
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
  view, a transcript viewer…), which herdr cannot tell apart. Hence
  verification rather than trusting either signal alone.
- Probe 2 (2026-10-14, a disposable haiku session in a tab created and closed
  for the test): during a long foreground Bash computation (`working`),
  `[inbox] Q9103 …` typed at 15:46:44 appeared at once as `queue-operation
  enqueue` + `attachment queued_command` (so verification passes within
  `VERIFY_SECONDS`). It was dequeued and answered when the tool finished at
  15:47:56.
  - A background subagent did not reproduce the long `working`: the main turn
    ended and herdr reported `done`.
  - The workflow wait itself ("Waiting for 1 dynamic workflow") couldn't be
    reproduced on haiku. The live Q45 record above is the evidence that it
    accepts input.


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

## The asking agent's pane moved (0.10.4)

Delivery is keyed by the pane a question was asked from. When that pane id has no agent any
more, or holds another conversation, the answer used to be marked "agent gone" after the
retries, even if the conversation still ran in another pane.
- Live 2026-10-16, Q155: pane `w6:p48` became `w6:p57` when it moved, and the answer was
  never delivered.

Now, before recording "no agent" or "conversation changed", delivery looks the conversation up
among herdr's agents (`agent list`). Continuations are followed on both sides.
- **Exactly one pane runs it:** the question moves there (`pane_id`, `tab_id`), pending
  "moved: the asking agent is now in pane …", and the next pass delivers it.
- **None, or more than one:** never a guess. The old outcome stands.
- **Already marked gone:** the worker re-checks answers marked "agent gone" within the last
  day, still unseen, of its own herdr server, and moves them the same way (one `agent list`
  per tick, only while such answers exist).

## Claude Code session continuation (0.10.3)

**Live, 2026-10-16** (pane "Ghostpaw frontend v2 rework"). Claude Code continued the
conversation from session `a58e3f68…` into `06acc80f…` **in the same process**.
- The old transcript ends with
  `{"type":"continued-in","sessionId":"<old>","continuedInSessionId":"<new>"}`, followed only
  by bookkeeping records (`last-prompt`, `cost-state`). Nothing in the new file points back.
- herdr later reported the **old** id again, and so does the process env
  (`CLAUDE_CODE_SESSION_ID`). Hooks get the new id in their payload.

That broke four things:
- New questions were filed under the old id, so delivery was verified against the dead transcript.
- Answers to questions filed under the new id were refused as "conversation changed", because
  herdr's id did not match.
- The new session's rewake watcher ended at its first pane check.
- The old session's watcher stayed alive (herdr's old id "matched"), kept stamping
  `watch_seen_at`, and held idle typing back for old-id questions.

**Fix: follow the record.** `transcript.follow(session)` / `chain(session)` read the session's
transcript. If its last `continued-in` is followed only by bookkeeping, they move to the named
session, repeatedly, without cycles.
- **Ids from transcripts are followed only as canonical UUIDs.** `find` itself accepts only
  `[A-Za-z0-9_-]`. These ids become part of a glob path, so `*`, `?`, `[`, slashes,
  backslashes and dots never reach it (review #41).
- **Reads are incremental:** a growing transcript is read only where it grew since the last
  look; a cold look reads the last 64 KiB. Both lookup caches are bounded (LRU, 256 entries).
- `herdr.live_agent` returns the followed id as `session_id` (herdr's own as
  `reported_session_id`). Delivery's conversation check, the watcher's pane check and the idle
  gate all compare against the live conversation. `resolve_caller` files new questions under it.

**Rows keep the session they were filed under.** That session is part of a remote question's
identity (`qkey`), so rewriting it would make an answer already in a remote outbox be refused
(review #41 High 1). Instead the worker records each continuation (`sessions.continued_in`, every
tick, idempotent), and everything that resolves "this session's rows" takes the session's
**family**: itself plus every recorded session that continued into it.
- That covers ownership (list_my_questions, check_answers, alerts, withdraw).
- It covers the hook and the rewake watcher's candidates, including the read-only fast path
  (a recursive CTE).
- Rows filed under the old id later, by a stale process, are covered without another pass.

**Verification reads the whole chain.** Every check for "did it already land?" looks in every
transcript the conversation lived in: the filed session and each session it continued in.
- That covers delivery's pre-type check, the hook's own pre-send check, the hook-send
  verification and the reconcile pass.
- So an answer that landed in the OLD transcript just before a continuation is found there
  and never sent again (review #41 High 2).

**A stored continuation only nominates.** `family()` includes a predecessor only while its
transcript **still** ends in `continued-in` to this session (cached, incremental read), both in
the store and in the hook's read-only fast path. So nothing derived from `sessions.continued_in`
can be stale (review #41 final pass).

**A resumed old session** (the old id used again after its `continued-in` record) is not
followed: the record no longer ends that transcript, and it leaves the new session's family at
once. Its rows are again its own, and a pane
showing the old conversation gets them. That is the safe direction: at worst an answer waits
as "conversation changed".

The old session's watcher ends at its next pane check: the pane now resolves to the new
session.

## Never type onto a draft; a delivery after other text still counts (0.10.1)

**Live, 2026-10-16 (Q124/Q125, a pre-0.9 session, so the typing path).**
1. The worker typed the batch into a working agent's pane after the busy grace. The input box
   still held the operator's unsent text, so Claude Code appended the batch to it and
   submitted both as one user message: `<draft>[inbox] answered — Q124 …: … · Q125 …: …`.
2. The matcher wanted our header at the start of a record, so it did not count this landing.
3. The answer went back to "retrying when the agent is not busy". The operator pressed
   **Send now**, its pre-check missed the merged record for the same reason, and it typed
   Q124/Q125 a second time.

The focus rule (wait while the operator is in that pane) could not help: the operator had
typed the draft and moved on, so the pane was not focused when the worker typed.

**1. Never type onto unsent text.** Before each typed message to a **Claude** agent,
`herdr.composer_state` reads the pane (`--format ansi`) and finds Claude Code's input box.
That is the lines between the last two `───` borders; the top border may carry a session tag,
as in `───… node ─`. Its first line starts with `❯`.
- `empty`, recognised positively (review #37): the `❯` line is blank, or holds exactly the
  dim placeholder `Try "…"` (one non-dim cell allowed, for a drawn cursor), and the box has
  no other line. Typing proceeds.
- `draft`: anything else. That covers typed text, wrapped or multi-line text, and pasted-text
  or image chips (`[Pasted text #1 +28 lines]`, `[Image #1]`, captured, plain), even if dim.
  The answer stays pending ("the agent's input box holds unsent text") and is retried.
- `unknown`: no box recognised. The answer is not typed either ("cannot read the agent's
  input box").

This applies to Send now as well, which also raises a notification so the operator knows why.
Other agents (Codex etc.) are **not checked** and are typed into as before. The same draft
hazard exists there, but their input boxes have not been captured, and an unrecognised box
must not stall them (review #37 L3).

**When the box stays unreadable**, for example after a Claude Code UI change, Claude panes stall
on typing. Hooks still deliver for 0.9+ sessions. After 60 s of `unknown` for a pane, the
worker raises **one** distinct notification, "<tab>: cannot read the agent's input box", so the
cause is visible. It is re-armed once the box reads again (#37 L2).

**Accepted residual (#37 L4):** a window remains between the read and the keystrokes. Hitting
it takes a second attached herdr client typing into that pane; the popup paths hold the
keyboard, and the worker re-checks focus just before typing. **The read→type
race:** the operator can start typing between the read and the keystrokes. Then our message
is again merged into theirs. Fix 2 below matches that merged record, so it is counted once
and never typed a second time. Fixtures are captured
from Claude Code 2.1.301 (`tests/fixtures/composer`).

**2. Count a delivery that follows other text.** The matcher looks at the record's start and
at every later `[inbox] ` in it. A later start counts only with full evidence:
- after typing (`landed_since`/`inbox verify`), the exact typed message from there;
- for the pre-type check and the reconcile (`landed_kinds`), our header naming the id plus
  that question's exact answer line or message.

Our header alone, or a `Q<n>` mention, never counts. With this, the incident's first landing
is verified, and Send now's pre-check records it instead of typing it again.

## Hook delivery to busy Claude sessions (0.9.0)

Typing into a pane is behind most delivery failures: a busy agent queues the text or loses it
to a focused panel or dialog, and the transcript check then has to catch the miss. Claude
Code can take text into a running conversation **without keystrokes**: a hook's
`additionalContext`. So delivery becomes per agent. Claude Code sessions running this plugin
version get answers through their own hooks while they work. Typing stays for idle agents, for
Claude sessions started before the update, and for other agents (Codex etc.).

**Probed on Claude Code 2.1.301 (moneylust, headless haiku, 2026-10-16).** A
`PostToolBatch` hook's and a `Stop` hook's `hookSpecificOutput.additionalContext` both reach
the model; the Stop one continues the turn (as "Stop hook feedback", not an error). The
transcript records each as

```json
{"type": "attachment", "attachment": {"type": "hook_additional_context",
  "content": ["[inbox] Q7 answered — …"], "hookEvent": "PostToolBatch", …}}
```

(plus a `hook_success` record whose `stdout` holds the raw JSON, which the matcher ignores).
The model sees it as `<system-reminder>PostToolBatch hook additional context: [inbox] Q7 …`.

**Events.**
- `PostToolBatch`: once per batch of tool calls, before the next model request. That is the
  point where a `PostToolUse` context would also land (next to the tool results). PostToolUse
  fires once per tool, concurrently for parallel calls, so it would start the hook several
  times for one model request. So the inbox registers PostToolBatch only.
- `Stop`: at turn end, `hookSpecificOutput.additionalContext` (non-error feedback). Claude
  continues and acts on the answer instead of going idle. Claude Code's
  8-continuation cap and `stop_hook_active` stay in force. Each answer is delivered once,
  so this cannot loop.
- Hooks firing inside a subagent or teammate (`agent_id` in the input) deliver nothing: the
  answer must reach the session that asked.

**The hook** (`hook.deliver`, stdlib only like the other hooks, always exit 0):
0. Liveness: stamp `sessions.hook_seen_at`, at most once a minute. The check is a read-only
   connection; a writable one opens only when the stamp is over 60 s old (0.10.0).
   (`sessions.hook_delivery`, a capability flag from 0.9.0's first head, is neither read nor
   written any more; the column stays because columns are never dropped.)
1. Fast path, read-only with a 0.2 s timeout: any answered, undelivered, unacknowledged
   question of this `session_id` that is not at the miss cap and whose agent is not gone (the
   same filter as `hook_candidates`)? Usually not: exit. Nothing else is written per batch.
2. Claim each with the usual CAS (`sending:<time>:<token>`). A row the worker is typing or
   verifying is skipped.
3. Already in the transcript (a late typed landing)? Record it verified, as `deliver_batch`
   does, and skip it.
4. Record the claimed batch as **hook-sent**: `delivered_at` set, `question_ext.hook_sent_at`
   set, not acknowledged. Only then print the context. Text and batching are the same as
   typing (`delivery_messages`: one message per firing; the rest stay for the next firing),
   so the transcript matcher and `header_ids` keep working.
5. Remember the session's transcript path.

**Verification** (the delivery worker, every tick). For a hook-sent row not yet verified, the
worker looks in its session transcript, where the matcher now also reads
`hook_additional_context` content. If found, the row is verified and acknowledged; for a
long-answer reference it is verified but left unacknowledged, as with typing. If it is not
found within `HOOK_VERIFY_SECONDS` (45 s), the row goes back to pending, as a
compare-and-set on the `hook_sent_at` that was judged, so a stale read never wipes a newer
send (review L1). Each worker verifies only its own herdr server's rows. The row goes
(`hook: not seen in the conversation`) and `hook_misses` is counted. After `MAX_HOOK_MISSES`
(2) the hook leaves that answer to typing.

**The worker no longer types into a working session whose delivery hook is live.** It
applies when herdr reports `working` and the session's `hook_seen_at` is less than
`HOOK_LIVE` (10 min) old. That is liveness, not capability (review M1). A session started
with 0.9.0 whose hooks stop firing still gets 0.8's busy rules: the plugin updated or
disabled mid-session (its hook commands point at the old root and fail silently), no
`PostToolBatch` in that Claude Code, or herdr misreporting an idle session as working. So
does a turn whose tool call outlasts `HOOK_LIVE`. The row stays pending
(`busy: delivered at the agent's next tool call or turn end`); there is no busy grace and no
busy typing. `idle`/`done` still type (no hook fires in an idle session). `unknown` keeps
0.8's rules. The operator's "Send now" still types.

**Idle sessions (next: asyncRewake, review L3).** Waking an idle session to take its answer
keystroke-free must use the same liveness gate as the busy rule. The worker leaves an idle
session to its watcher only while that watcher is provably alive (it holds a per-session
lock) and the session's hook is live; otherwise it types as before. Both paths claim before
acting and check the transcript first, so a race between them stays once-only.

**Once and only once.** Hook, typing and `check_answers` meet on the same row:
- Each sender holds the CAS claim until it records. A hook-sent row has `delivered_at` set,
  so neither `claim_delivery` nor the hook's fast path picks it up again.
- `check_answers` acknowledges whatever it returns; the hook skips acknowledged rows.
- The only path to a second delivery is the 45 s verification reset. It happens only when
  the transcript does not show the first one, the same rule as typing.

## Waking idle Claude sessions without typing (0.10.0)

0.9.0 covered working sessions. This covers idle ones: a Claude Code session at rest gets
its answer without a keystroke, through an **asyncRewake** hook. A command hook marked
`asyncRewake: true` runs in the background; when it exits 2, Claude Code shows its output
to the session as a system reminder and **starts a turn**, even when the session is idle.
After this, typing remains only for other agents (Codex etc.), Claude sessions started
before the update, sessions whose hooks stopped firing, and "Send now".

**The watcher** (`rewake.py`; `Stop` hook `rewake-watch`, `asyncRewake: true`, timeout
43200 s):
- Every turn end arms one watcher for its session. It polls the database read-only every 2 s
  for an answer of this session that is neither delivered nor seen.
- It acts only while herdr reports the pane's agent `idle` or `done` *with this session*.
  It looks at the pane every 45 s even with nothing pending, so a watch that a `/clear` or
  `/resume` left behind ends within a minute (review #30 M1). If herdr names no session for
  the pane, it never wakes; after 3 minutes of that the watch ends (M1b).
  - `working`: the 0.9.0 PostToolBatch/Stop hooks carry the answer.
  - `blocked`: never disturbed.
  - The pane now holds another conversation: the watch ends.
- To deliver, it takes the same path as `hook.deliver`: CAS claim, transcript check for a
  late landing, record hook-sent. Then it prints the `[inbox] …` text and exits with a
  private code (3), which the `hooks.json` wrapper maps to 2. Nothing else can exit 2: the
  interpreter's own errors are 1, and the wrapper turns everything but 3 into 0.
- **One per session, never leaked:**
  - Each new watcher writes its token to `rewake/<session>.owner`; an older one that sees
    another token exits within a second.
  - It also exits when its Claude process is gone (the nearest `claude` above it, checked
    each poll), and just under the hook timeout (about 12 h).
  - While alive it holds `rewake/<session>.lock` (flock).
  - Live on moneylust: exactly one watcher across several turns, and none after `/exit`.

**Cost per idle session (review #30 L3).** One sleeping `python3 -I` process (plus its `sh`
wrapper) for as long as the session is idle, up to about 12 h. Measured: 26 MB RSS, of which
about 12 MB is private (the rest is shared libraries). It does a read-only SQLite look every
2 s, one herdr call every 45 s, and at most one write a minute. With ten idle agents that is
about 120 MB private memory and five database looks a second, all read-only.

**Waking the pane the operator is looking at (review #30 L1, intended).** Unlike typing, the
watcher does not wait while the operator has the pane focused. A wake is keystroke-free, so
nothing merges into what the operator is composing. They see the agent take the answer and
start a turn, which is what they asked for by answering.

**The worker's idle rule.** An idle session is left to its watcher (pending, "idle: the
session's own hook wakes it with the answer") only while both hold. These are the watcher's
own evidence, kept apart from the working gate (review #30 M1):
- the watcher is provably alive: it holds the lock;
- its own stamp `sessions.watch_seen_at` is less than `HOOK_LIVE` old; the waiting watcher
  writes it at most once a minute.

The WORKING gate reads only `hook_seen_at`, which only PostToolBatch/Stop firings write, so a
live watcher never keeps a working session on hook delivery when those hooks have gone
silent. The watcher's own deliveries do not stamp `hook_seen_at` either.

This is the same liveness gate as the busy rule. Otherwise, after a plugin update the old
root's hooks no longer start and no watcher holds the lock; or past the watcher's max age;
or with answers the hooks missed twice: the worker types as before.

**Once only.** The watcher, the PostToolBatch/Stop hooks, typing and `check_answers` all go
through the same claim and transcript checks, and a hook-sent row is verified (or re-pended
after 45 s) by the worker exactly as in 0.9.0.

**The transcript.** The woken text lands as a `user` record with
`origin.kind = "task-notification"`. Its content wraps our text (captured on Claude Code
2.1.301):

```
<task-notification>
<summary>Stop hook feedback</summary>
</task-notification>
<system-reminder>
Stop hook blocking error from command "Stop": [inbox] Q1 answered — …: amber

</system-reminder>
```

The matcher finds our own `[inbox] ` header anywhere inside a task-notification record and
takes it up to the end of its paragraph (review #30 L2). It does not rely on Claude Code's
label wording, so a reworded wrapper still verifies; `header_ids` and the segment checks work
unchanged. The "blocking error" label is Claude Code's wording
for any asyncRewake output. It is cosmetic: no error is shown to the operator.

**Consent caveat.** Claude Code tells the model that a woken message is an automated system
notification, **not user input**, and that it must not be treated as the user's approval or
consent. In plain words: an answer delivered this way reaches the agent, and the agent acts
on it as information. Live: it asked which word to write, got "zebra" this way, and wrote it.
But an agent may refuse to treat it as the operator's **go-ahead** for something risky
("deploy to prod? → yes"). That matches the existing rule: blocking or destructive
confirmations are asked in chat, not through the inbox. Hook `additionalContext` (0.9.0) is
also a system reminder, so the same holds for answers delivered to busy sessions. Typed
answers (other agents, "Send now") still arrive as user messages.

## Deep links: opening the popup on a target (0.6.0)

The integration API for other plugins (e.g. herdr-panorama). It is
`inbox open [--question Qn | --group G | --pane P | --session S]`, run as
`uv run --project <plugin_root> inbox open …`; `herdr plugin list --json` gives
`plugin_root`. `herdr plugin pane open` cannot pass arguments to the pane
command, so `deeplink.py` hands the target over through the state dir:

- **Write:** `open-request.json` = `{kind, value, created_at, nonce}`, via a temp
  file and `os.replace` (atomic). The newest request wins, and the popup is then
  opened as by the `open` action.
- **Consume:** on start, the popup renames the file to a unique name (atomic, so
  two popups starting together cannot both take it), reads it and deletes it.
  It honours the request only if it is ≤ `REQUEST_TTL` = 5 s old. Garbage or
  stale requests are dropped.
- **Resolve:**
  - a question → its group, opened on that tab;
  - a group → that group;
  - a pane or a session → its oldest open group.

  A target that is gone means the list plus a notice ("Q12 is no longer open").
- **Which popup:** `herdr.open_popup` always opens `leavemyyard.herdr-inbox`,
  with `--cwd` set to the inbox's own checkout (found from the package's
  location, or else `plugin_root` from `herdr plugin list`). It never uses
  `HERDR_PLUGIN_ID`/`HERDR_PLUGIN_ROOT`, because when another plugin's action
  runs `inbox open`, those name the caller.
- **Handshake:** `inbox open` passes the request's nonce to the popup it opens
  (`--env INBOX_OPEN_NONCE=…`). A popup takes only the request carrying its
  nonce, so a popup opened by hand never takes someone's pending request.
- **Already open:** a running popup holds an exclusive flock on `popup.lock` in
  the state dir. `inbox open` checks it and, if held, exits 3 with "already
  open" and writes nothing.
- **Safety:** a deep link is an event from outside, and a popup takes all
  keyboard input. So a deep-linked popup starts **held**: every Enter, digit
  and toggle is refused through `_armed`, the same gate as `ARM_SECONDS`. The
  hold ends only on Tab/→, the keys that also release the empty popup's
  arrivals, and only once `RELEASE_GRACE` = 0.5 s has passed since the popup
  appeared. ↑/↓/j/k don't release it, because a shell's history (↑, Enter)
  under a popup that just appeared would. The releasing key does nothing else.
  A hint line says so. After that the views arm as usual.
- **`--no-hold` (0.6.2, the caller decides):** `inbox open … --no-hold` writes
  `"hold": false` into the request. Only an explicit `false` lifts the hold, so
  a request without it (or with garbage there) stays held. Such a popup opens
  like a normal one (`ARM_SECONDS`, then Enter works). Callers pass it only when
  an explicit operator key or click opened the inbox. There is no config and no
  caller list: the operator chose the plain flag. The view is opened as a choice (`direct` is
  false), so Esc returns to the list. The README requires callers to run
  `inbox open` only on an explicit operator action. Nothing changes for a popup
  without a request.

## Responsiveness (0.6.3)

Operator report: "sometimes sending inbox messages blocks all inputs and everything just
lags". This was measured on an isolated herdr (own HOME/XDG/socket/DB under `.scratch`), with
a stand-in agent reported idle, so sends really type. Before = main @ d56f785, after = this
change:

| | before | after |
|---|---|---|
| popup: Enter on a lone question → popup closed (send lands at once) | 386 ms | 82 ms |
| same, the answer never lands (15 s transcript check) | **15 373 ms** | 82 ms |
| `ask_operator` round trip (stdio MCP server), p50 / max | 23 / 121 ms | 17 / 56 ms |
| `ask_operator` while another process holds the tab-label lock | **4 030 ms** | 17 ms |
| herdr API latency for another pane during a send, p95 | 2.0–2.2 ms | 1.9–2.3 ms |

- **Popup:** it records the answer in a thread (one DB write) and hands delivery to a
  detached `inbox deliver <ids> [--force]`, which outlives the popup; claims serialize it.
  That sender keeps the worker's rule and never types into the pane the operator is in;
  only `--force` (Send now) skips it.
  - The exception is the pane the popup returns to: the operator came from it and will
    type there next. Its answers are typed in the popup's thread before it closes, while
    the popup still holds the keyboard (~0.3–0.6 s: `herdr agent prompt` ≈ 300 ms). Only
    the transcript check is handed off, to a detached `inbox verify` that records the
    outcome under the same claims.
  - Send now on an answer another process is delivering says "already being delivered".
  - Queued background jobs are drained (≤ 1 s) when the popup or an MCP server exits.
  The popup then closes or returns to the list at once, and Esc waits only for that record
  step. Before, the popup waited for the whole delivery, including the 15 s verify window.
  - Herdr calls leave the UI thread: tab-label lookups refresh in a background thread
    (the popup shows the last known label meanwhile), "go to agent" focuses from a
    thread, and markers after a dismiss go to the background queue.
- **Agent side:** the MCP server calls `service.enable_background()`. Tab-label updates
  (a lock that can wait 10 s) and herdr notifications run on one background thread
  after the tool has answered. A failing job (even a `BaseException`) is logged and the
  thread carries on. Jobs still queued when a short-lived server exits (`claude -p`) are
  lost; they are cosmetic, and the next change or `sync-tokens` fixes the labels.
- **No stranded answers:** the popup's record step ensures the delivery worker before
  spawning the sender. A popup killed in between leaves the answer to the worker.
- **First calls on a fresh MCP server** (cycle review: 0.8–1.4 s). Each tool call, and each
  background tab-label job, opened a `Store` that re-ran the schema script and the reconcile
  pass. Those are write transactions, so several connections in one process, plus the
  freshly spawned worker, waited on SQLite's busy handler for up to ~1 s.
  - The database is now stamped (`PRAGMA user_version` = a hash of the schema and of
    `Store._migrate`'s source), so a reopen does no schema work. The migration runs under
    `BEGIN IMMEDIATE`, checks the stamp again once it holds the lock, and stamps in the
    same transaction. Several servers opening an old database together (an upgrade with
    many agent panes) serialize, and only the first one migrates.
  - Tool calls and background jobs open with `maintenance=False` and skip reconcile; the
    worker reconciles every tick.
  - Bench, `ask_operator` ms, before → after:
    - six calls on one server: 179, 952, 997, 61, 35, 31 → 83/95 (DB creation), then 7–27;
    - first two calls on five fresh servers: up to 342 and 537 → at most 25.
- **herdr:** a send never stalled herdr for other panes. `herdr agent prompt` itself
  takes ~300 ms for one message, and only that call waits on it.
- **`INBOX_TRACE=<file>`** (off by default) appends one JSON line per step to `<file>`:
  every herdr call, the popup's handlers, `ask_operator`, delivery phases and lock waits,
  each with its milliseconds and thread. Use it to investigate any future lag.

**Answers that can't be delivered (0.6.3).**
- A row whose "no agent" / "conversation changed" retries ran out is marked
  `agent_gone_at` once. The worker never polls it again and sends one notification ("the
  agent in <tab> is gone — the answer is kept; a new session can fetch it with
  check_answers"). The answer stays unseen for `check_answers`.
- Hiding a row only takes it out of the popup: a hidden pending answer is still delivered.
- Pre-0.6.1 rows with no server recorded are checked at most every 60 s, and called gone
  after 10 misses. Before, a gone pane's `agent get` failed on every 5 s tick.
- In the popup:
  - a gone row reads `⚠ Q77 agent closed — answer kept for check_answers`, and `d` hides
    it at once (no confirmation: nothing is lost);
  - pending rows read `⏳ Q75 answered · waiting to deliver: …`, so nobody tries to
    answer them again;
  - hidden rows are not listed.

## Alerts (0.8.0)

The operator saw tabs at herdr's "needs you" with no question in the inbox. Cause: the agent
sat at a blocking prompt (permission dialog, AskUserQuestion, plan approval) where it cannot
call any tool, so it could not have asked. Two additions:

- **`alert_operator(title, message?, severity, asked_by?)`**: a "please check X" with no answer
  expected. Table `alerts` (side table; `questions` keeps its columns): `kind` `agent` |
  `blocked`, `severity` `info` | `warning` | `urgent`, `status` `open` | `acked` | `withdrawn` |
  `cleared`. Open alerts count in `Store.open_counts` (badge, `$inbox_q`, radar). Ids show as
  `A<n>`. One notification on raise. An open alert with the same title from the same session
  is returned instead of a second one.
- **Blocked agents** (`alerts.scan_blocked`, every worker tick): one `herdr agent list`. herdr
  documents `blocked` as "recognized an approval or question UI". A pane blocked for
  `BLOCKED_AFTER` (15 s, so a quickly answered permission prompt never alerts) gets a
  `kind='blocked'` alert. When each pane's prompt was first seen lives in `blocked_panes`
  (per herdr server), so a worker restart or hand-over does not restart the 15 s clock. It is
  `cleared` only after `CLEAR_AFTER` (10 s) and two ticks of
  another status, so a flickering detection within one prompt neither clears nor re-notifies;
  an agent gone from the list clears at once. Dedupe and dismissals are scoped by
  `herdr_server` (pane ids repeat across servers). Nothing is ever typed into such a pane;
  `herdr agent prompt` refuses blocked agents anyway (`agent_blocked`).
- **The worker no longer idles out in normal use.** It stays alive while herdr reports any
  agent (it watches their blocked state), so with agents running it always runs: one per
  herdr server. Its cost per tick: one `herdr agent list`, one `tab get` per newly blocked
  pane, plus the existing delivery tick. `WORKER_IDLE_EXIT` now applies only to a herdr with
  no agents and nothing pending.

**Acknowledgement is not typed back** (decided default). Typing into an agent costs a turn and
can land at a prompt; the agent does not need it to continue. `check_answers` returns
acknowledged alerts once (`reported_at`), `list_my_questions` shows their status.

**Popup: alerts never change what a habitual key does to a question** (reviews M1, H1). An
operator types "1 Enter 1 Enter", "1 g", "Enter Enter" without looking; with an alert in the
way, those keys could acknowledge it unread or, for a blocked agent, focus its pane and hand
the next key to its approval prompt (where the highlighted choice is usually "Yes"). So:

- Alerts are listed first (local, then remote, most severe first) but **unnumbered**: digits
  address question rows only, numbered exactly as before 0.8; alerts are reached with ↑/↓.
- The initial cursor is on the focused pane's question group, else at rest. The only question
  still opens directly; after answering, the popup closes when no question is left (open
  alerts keep it open only after an alert action).
- The `AlertView` has no numbered options and no Enter action: `d` acknowledges (dismisses a
  blocked alert, which holds until that prompt ends), `g` goes to the pane, Esc goes back.
- **`g` to a waiting agent must be deliberate:** for an alert (list row or view) `g` acts only
  `ALERT_GO_GRACE` (0.5 s) after the previous key of any kind (a g too: a held, auto-repeating or double-tapped g
  never passes) and the last view change; for a
  question, the popup asks herdr for the pane's live status first and applies the same rule
  when it is `blocked` (covers an agent alert or question whose agent has since hit a prompt).
- `d` on an alert row in the list acknowledges without a confirmation: nothing is lost. Closed alert rows
become inert placeholders like every other row.

**Remote.** `inbox export` carries `alerts` (open ones and those closed in the last hour,
≤200, whole on every pull; the change-feed cursor is for questions only). The mirror table
`remote_alerts` is replaced per pull, except that an alert with a queued `ack_alert` stays
acknowledged. Acknowledging enqueues `ack_alert` in `remote_outbox` with the alert's `akey`
(`sha256(db_id|alert|id|created_at|title)`); `inbox apply` checks `db_id` and `akey`, records the
`request_id` in the same transaction, and rejects a closed alert with its status. Alert ids and
question ids are separate namespaces: the outbox FIFO keys on `(is_alert, id)`.

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
  files included, and a copied `.venv` has no interpreter. The MCP server
  therefore runs `uv run --frozen --no-dev --project ${CLAUDE_PLUGIN_ROOT}` with
  `UV_PROJECT_ENVIRONMENT=${CLAUDE_PLUGIN_DATA}/venv-<version>` (the server's
  `env`), so the environment lives in the plugin's persistent data dir and the
  copy is only read. One venv **per version** (0.5.3): the data dir is kept
  across updates while sessions started earlier keep the old root, so two
  versions sharing one venv would re-sync it under each other.
  `tests/test_plugin_copy.py` builds such a copy (broken `.venv` included) and
  runs the commands exactly as the manifests declare them.
- **Hooks never exit non-zero and never run uv (0.5.3).** Claude Code gives exit
  code 2 a meaning of its own. For SessionStart and SubagentStart it cannot
  block, but the run is a hook error: a "hook error" notice is shown, Claude
  doesn't see it, and the hook's `additionalContext` (the inbox rules) is lost.
  Other non-zero codes are non-blocking errors too. `uv run` exits 2 on its own
  failures: in a sibling plugin's review, 9 of 48 concurrent runs did so with two
  versions sharing one venv. The hooks are therefore exec-form
  `sh -c 'python3 -I "$1" "$2" 2>/dev/null; exit 0' herdr-inbox-hook
  ${CLAUDE_PLUGIN_ROOT}/hooks/run.py <event>`. `hooks/run.py` imports the
  package from `src/`; the hook's import chain is stdlib only. `-I` keeps out
  user site, PYTHONPATH and the cwd. Only a python3 older than 3.11 hands over to
  `uv run` (with the version's venv), forwarding output only on success.
  `tests/test_hook_launch.py` covers a fake failing `uv`, a failing `python3`,
  bad arguments, the old-Python fallback and two versions' hooks running
  concurrently. The fallback runs uv in its own process group and kills the
  whole group on timeout, so a grandchild holding stdout cannot outlive it.
- **The rules come out first.** The SessionStart hook builds its context and
  prints it before anything that writes. The compact/resume pending-question
  lookup is a read-only connection with a 0.2 s timeout; if it fails, the hook
  prints the rules alone. Recording the transcript path comes after the output,
  in its own `try` with a 0.5 s timeout. It is an optimisation: verification
  falls back to the projects-dir glob. With the DB exclusively locked, the rules
  still print in ~0.2 s plus interpreter start.
- **What a per-version venv does not cover.** A plugin installed from a
  local-path marketplace (as here) is loaded in place and not pinned by
  `version`, so `${CLAUDE_PLUGIN_ROOT}` is the live checkout. A `git pull`
  changes the source under MCP servers that are already running, and a module
  they import lazily (e.g. `transcript`) loads the new code into an old process.
  Restart Claude sessions after pulling.
- **Old venvs are cleaned up.** The first thing `inbox mcp` does, before any
  heavy import, is take a shared `flock` on `<its venv>/.inbox-in-use`. It holds
  that lock for its lifetime and refreshes the file's mtime. If the venv is
  already gone by then, the server exits rather than run from a deleted tree.
  Claude Code does **not** restart a stdio MCP server that exits: per its MCP
  docs, only remote servers are reconnected automatically. The inbox tools then
  stay unavailable in that session until the operator reconnects the server
  (`/mcp` → Reconnect, after which uv rebuilds the venv) or starts a new
  session, and the stderr line says so. The race needs an old-version server
  starting at the same moment as a newer one's cleanup, against a venv whose
  lock file has been untouched for an hour.
  On start, a background thread (`venvs.py`) then removes a direct child of
  `${CLAUDE_PLUGIN_DATA}` named `venv-<numeric v>` only when all of these hold:
  - v is older than the running version;
  - neither the dir nor its lock file has changed for an hour;
  - the thread can take an exclusive non-blocking flock on the lock file.

  Removal first renames the venv to a unique `.removing-…` name, so concurrent
  cleanups can't both act, and then deletes it. Leftover `.removing-*` dirs
  older than an hour are swept. Venvs without a lock file (the pre-0.5.3 shared
  `venv`, and any from a version older than the lock) are never removed
  automatically, because an mtime says nothing about use; `inbox clean-venvs`
  removes them on request. The kernel answers "in use", so path spelling and
  `ps` don't matter. One assumption remains: a filesystem that only pretends
  to support flock (some FUSE mounts) would defeat it, whereas one that rejects
  flock fails safe.
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
