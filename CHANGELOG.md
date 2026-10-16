# Changelog

## 0.10.0

- **Idle Claude Code sessions are woken with their answers, without typing.** A `Stop` hook
  with `asyncRewake` arms one watcher per session at each turn end. When an answer for that
  session arrives and herdr shows the agent idle or done, the watcher claims the answer,
  records it and exits with the `[inbox] …` text. Claude Code then starts a turn with it.
- **One watcher per session:**
  - A newer watcher supersedes the older one.
  - A watcher exits with its Claude process, after about 12 h, or when the pane changes
    conversation. It checks that every 45 s even with nothing pending, so `/clear` and
    `/resume` leave no watcher behind. It never wakes a working or blocked agent, or a pane
    that herdr names no session for.
  - While alive it holds a per-session lock, and the delivery worker leaves an idle session to
    it only while the lock is held and the session's hook is live. Otherwise the worker types,
    as before.
- The transcript matcher reads the woken `task-notification` record (captured on Claude Code
  2.1.301).
- Typing now remains only for other agents, pre-update sessions, sessions whose hooks stopped
  firing, and "Send now".
- **Consent:** Claude Code marks hook-delivered answers as automated notifications, not user
  input. Agents act on them, but may not take one as your go-ahead for something risky. Keep
  asking for those in chat (README, "Delivery").
- The delivery hook's once-a-minute liveness check now reads through a read-only connection,
  and opens a writable one only when the stamp is stale. `sessions.hook_delivery` is no longer
  written; the column stays, unused.
- **Restart Claude Code sessions** to pick up the new hook.

## 0.9.0

- **Answers reach a busy Claude Code session without typing.**
  - New `PostToolBatch` and `Stop` hooks hand a session its waiting answers as hook
    `additionalContext`, with the same `[inbox] Q7 answered — …` text. The answer lands at
    the agent's next tool batch, or at turn end, where Claude continues with it.
  - Claude Code records the context in the transcript as a `hook_additional_context`
    attachment. The delivery check reads those records, so the answer is acknowledged once it
    shows there.
  - If it does not show within 45 s, the answer goes back to pending. After two such misses
    it is typed as before.
  - Each answer is claimed (CAS) and recorded before the hook prints it, so the hook, typing
    and `check_answers` deliver it only once.
- The delivery worker no longer types into a **working** Claude session whose delivery hook
  fired in the last 10 minutes. It waits for the hook instead: no busy grace and no busy
  typing. A session whose hooks stop firing (plugin updated or disabled mid-session) falls
  back to 0.8's busy rules. Idle and done
  sessions, sessions started before this update, and other agents are typed into as before.
  "Send now" still types.
- **Cost:** a session with no waiting answer pays one read-only query per model step, plus
  a liveness write at most once a minute. The hook takes about 40 ms there, python start
  included.
- **Restart Claude Code sessions** to pick up the new hooks. Sessions already running keep
  0.8 delivery.

## 0.8.0

- **Alerts: "please check X" without a question.** A new MCP tool
  `alert_operator(title, message?, severity=info|warning|urgent, asked_by?)` returns at once
  with a receipt (`A7`). The alert raises one herdr notification, counts in the pane's ❓N badge
  and `$inbox_q` (so in the radar sidebar too), and heads the popup list (`⚑ A7 · urgent · …`,
  most severe first, unnumbered) until the operator acknowledges it with `d`; `g` goes to the
  agent. Alerts never change what habitual keys do to questions: digits still number only the
  questions, the cursor starts on the focused pane's question, the only question still opens
  directly, and an alert view has no Enter or digit action. Nothing is typed back into the agent's pane: `check_answers` reports
  acknowledged alerts once (`alerts_acknowledged`) and `list_my_questions` lists them. The
  agent can `withdraw_alert(id)`. The same title raised twice returns the open one.
- **Agents stuck at a prompt become alerts.** An agent herdr reports as `blocked` (it shows an
  approval, AskUserQuestion or plan prompt — herdr's "needs you") cannot call any tool, so it
  never asked. The delivery worker now checks `herdr agent list` every tick: an agent blocked
  for 15 s gets an automatic alert "`<tab>` is waiting for you" (`⏸`, one notification), and
  the alert clears itself once the status has changed for 10 s (two ticks), so a flickering
  detection does not re-notify. Only a deliberate `g` (0.5 s after any other key) goes to such
  a pane, also from a question whose agent herdr reports blocked. The inbox never types into
  such a pane.
- The delivery worker now stays alive while herdr reports any agent (it watches their blocked
  state); before, it exited after 10 idle minutes.
- Remote machines: alerts travel in `inbox export` (open ones, plus those closed in the last
  hour) and show here as `moneylust:A3`; acknowledging one goes through `inbox apply`
  (`ack_alert`) with a request id, like answers. An older remote sends no alerts and is left
  alone.
- The SessionStart rules and the operator-questions skill tell agents to use `alert_operator`
  for "please check X" and to prefer `ask_operator` over AskUserQuestion.
- **Fix: `[E2E TEST]`-style brackets vanished from the popup.** `rich.markup.escape` leaves a
  tag that starts with an upper-case letter alone and Textual's markup then swallowed it, so a
  question titled `[E2E TEST] …` showed without its prefix. Every `[` is escaped now.

## 0.7.0

- **Remote herdr machines** (docs/REMOTE.md).
  - Questions from agents on saved herdr SSH machines appear in this machine's popup as
    `moneylust:Q12`. You answer them here, and the remote's own inbox delivers them there.
  - This machine's delivery worker pulls each enabled machine every `remote_sync_seconds`
    (default 5) over key-only ssh. It runs the remote's own `inbox export` and sends your
    answers, dismissals and Send now through `inbox apply`, with request ids so retries are
    safe. All data travels as JSON on stdin and stdout.
  - Each new remote question raises a notification here.
  - Offline machines are shown, never wait on, and answers to them queue for up to 12 h.
  - Deep links accept `--question moneylust:Q12`.
- `inbox remote install|upgrade <machine> [--ref]` installs or upgrades both plugins on a remote
  over ssh.
- **Note:** after this update the delivery worker pulls every machine `herdr machine list` shows as
  enabled, every 5 s (machines in parallel, each call capped at 10 s). For a machine without the inbox
  installed, the pull fails quietly and backs off (5 s up to 60 s).

## 0.6.3

- **Fix: the popup and agents no longer wait for deliveries, tab labels or herdr.** The popup
  records an answer and hands delivery to a detached `inbox deliver`, then closes at once. On
  an isolated herdr, closing went from up to 15.4 s (an answer that does not land) to 82 ms.
  `ask_operator` runs tab-label updates and notifications in the background: with the label
  lock busy, a call went from 4 s to 17 ms. Tab-label lookups and "go to agent" left the UI
  thread.
- `INBOX_TRACE=<file>` logs step timings (herdr calls, popup handlers, tools, delivery).
- The first `ask_operator` calls on a new session no longer take up to ~1 s. A reopened database
  skips schema work (it is stamped), and tool calls leave the reconcile pass to the worker.
- Answers whose agent is gone are marked once, notified once and no longer polled. The popup
  labels them "agent closed — answer kept for check_answers", and `d` hides them. Pending
  answers say they are already answered and waiting to be delivered. Hiding an answer only
  takes it out of the popup; its delivery continues.
- Before every typing attempt, delivery checks the transcript (when known) for the answer, so a
  sender killed mid-verify can't cause a second copy.

## 0.6.2

- `inbox open … --no-hold` opens a deep-linked popup like a normal one: 0.4 s arming, then Enter
  works, with no Tab/→ hold. The caller decides, and passes it only when an explicit operator
  key or click opened the inbox (herdr-panorama's `a`, say). Without the flag the popup stays
  held as before.

## 0.6.1

- **Fix: answers to long-lived sessions could go undelivered with nobody retrying.** Retries
  used to live in the asking session's own MCP server, so a session started before an update
  kept old (or no) retry logic.
  - One herdr-side **delivery worker** (`inbox worker`, a singleton) now retries, verifies and
    applies the busy/idle policy for every pane. It starts with herdr, from the popup, the CLI
    and new MCP servers.
  - An answer an old server typed and called seen, but which never landed, is sent again.
- An answer typed while the agent is busy and not seen waits for the next idle moment. The
  popup shows it at once as ⚠ "not delivered — agent busy".
- The worker never types while Claude Code's background-agents panel has focus: text typed then
  never reaches the conversation (tested on a throwaway session).
- Answers still undelivered after 5 minutes, or out of tries, raise one herdr notification: the
  agent can fetch them with `check_answers`.

## 0.6.0

- **Deep links:** `inbox open --question Qn | --group G | --pane P | --session S` opens the popup
  straight on that question or group, so other plugins can link into the inbox (`herdr plugin list
  --json` gives `plugin_root` for `uv run --project`). The target is handed over as a one-shot
  request in the state dir (atomic, newest wins, honoured within 5 s). If it is gone, the popup
  shows the list with a notice. README: "Opening the inbox from another plugin".

## 0.5.4

- **Fix: answers to an agent that stays "working" were held indefinitely.** herdr reports
  `working` for as long as Claude Code waits on a background workflow (hours), but Claude Code
  accepts typed input then. A busy agent now gets its answer after a 20 s grace, and the
  transcript check (at most 2 tries) confirms it landed. An agent at a prompt or dialog
  (`blocked`) still waits, since typed text would be swallowed.
- The popup says why an answer waits and when it goes (`⏳ Q45 agent busy — will send in 15 s`,
  `waiting: agent is at a prompt/dialog`), and a pending answer offers **Send now**.

## 0.5.3

- **Hooks can no longer fail the session start.** The SessionStart and SubagentStart hooks went
  through `uv run`, which exits 2 when uv itself fails, e.g. two plugin versions sharing one
  venv right after an update. Claude Code then shows a hook error and drops the inbox rules from
  the session. The hooks now run stdlib-only with `python3 -I hooks/run.py`, behind an `sh`
  wrapper that always exits 0. A python3 older than 3.11 falls back to uv.
- The MCP server's venv is per version (`${CLAUDE_PLUGIN_DATA}/venv-<version>`), so sessions on
  the old and new versions never re-sync each other's environment.
  On start, the MCP server removes older versions' venvs that no server holds a lock on.
  `inbox clean-venvs` also removes venvs from before 0.5.3, which carry no lock; it runs only
  when asked.
- A busy or locked inbox DB no longer drops the session's inbox rules. The rules are printed
  first; the pending-question list (compact/resume) and the transcript record give up after a
  fraction of a second.

## 0.5.2

- **herdr-radar sidebar, on demand:** `inbox radar-sidebar` and the herdr action *Inbox: show ?N in
  herdr-radar's sidebar* add the `$inbox_q` cell to every agent row of herdr-radar's managed
  sidebar block (radar has no slot for other plugins' tokens yet; hhdebb/herdr-radar#57). It edits
  only inside radar's markers, never past herdr's 16-token row limit, only if the file parses
  before and after, atomically with a backup, and steps aside once radar carries `$inbox_q` in
  `extra_tokens`. `--remove` takes it out again. Nothing runs it automatically; the popup shows a
  one-line hint when radar's rows lack the token (`radar_sidebar_hint = false` hides it).

## 0.5.1

- **Fix: answers could be lost when the asking agent was busy.** `herdr agent prompt` succeeding
  only means the keys reached the pane; in live use answers typed while the agent was mid-turn
  never reached the conversation, yet were marked delivered and seen. Now:
  - an answer is typed only when herdr reports the agent `idle`/`done`; while it works the answer
    is *pending delivery* (shown as `⏳` in the popup) and the asking session's retry loop (every
    10 s) delivers it at the next idle;
  - a delivery counts only once the text shows up in the session's transcript (a `user` record
    with string or text-block content, including a collapsed long paste, or a queued
    `queue-operation`/`queued_command`) within 15 s; otherwise it is typed once more, then left
    as ⚠ in the popup and unseen for `check_answers` (never typed every tick);
  - "seen" means the agent fetched the answer (`check_answers`/`list_my_questions`) or the
    delivery was verified, never just "sent"; the receipt and the SessionStart text point to
    `check_answers` as the fallback.
- Recovery: answers marked seen without verification (from the last 7 days on upgrade, and
  continuously from older 0.4/0.5.0 servers still running) are checked against the session
  transcript; only the ones not found there become unseen, so `check_answers` returns them
  (at most twice per answer). `inbox redeliver <ids>` delivers answers again.
- The SessionStart hook records the session's transcript path for verification.
- The popup shows each asking tab's current name (resolved from herdr, ❓ prefix stripped, cached
  for 5 s) instead of the label captured at ask time, which for an unnamed tab was just its number.

## 0.5.0

- `ask_operator` takes a `title`: a short name for the whole call (≤60 chars), strongly
  requested when asking several questions. The group list shows it as each row's main line
  (`❯ 1. Worker rollout settings · 4 questions · 1 drafted · 4m`), with the tab label, `asked_by`
  and header chips on the dim second line; inside a group it sits above the tabs. Untitled calls
  fall back to the single question's header, else its first line.
- Stored per group in a new side table (`question_groups`); the `questions` table is unchanged.
- Read views are versioned and never dropped, so sessions still on 0.4 keep working against a
  database 0.5 has opened (until they restart).
- The group list scrolls to keep the cursor's row visible; an untitled single question's
  notification shows its header and its text.

## 0.4.0

- Submit is per group (one `ask_operator` call). With exactly one open group the popup opens
  straight into it; with several it opens on a group list styled like Claude Code's option list
  (`❯ 1. Billing API · planner · 3 questions · 4m`, headers on a second line), with ⚠ undelivered
  answers as their own entries. Inside a group: its tabs and its own Submit; Esc returns to the
  list. `d` on the list dismisses a whole group (`y` confirms).
- Colours follow Claude Code's theme roles: `suggestion` for the cursor and focus, `success` for
  chosen answers (Review list, ✓), `warning` only for warnings.
- All 0.3 safety rules carry over to the list: arrivals are appended without moving the cursor's
  target, ended rows are inert placeholders until the operator moves, every view arms 0.4 s after
  it appears. The `+N new` marker is gone (arrivals are list rows). The list never wraps around,
  and a row that just arrived ignores Enter and digits for 1 s.

## 0.3.0

- The popup opens straight into Claude-Code-style tabs, one per open question (no list screen).
  Questions from the focused pane come first; one open question sends on Enter, several are
  drafted and sent from a Submit tab.
- No background event changes what the next key does: the current view is never swapped for
  another question, arrivals wait behind a `+N new` marker, and views arm 0.4 s after they appear.
- `g` goes to the asking agent's pane; `d` dismisses one question after a `y` confirmation.
- Delivery claims on every path, so a popup retry and the MCP retry loop never both send.
- `herdr-plugin.toml` gains a `[[build]]` step and runs with `--frozen --no-dev`; MIT license;
  CI; README rewritten for public release.

## 0.2.1

- The highlighted option is always scrolled into view.
- Sends run in a background worker; closing the popup waits for an in-flight send.
- Marker (token / tab label / lock) failures never fail `ask_operator` or an answer.

## 0.2.0

- AskUserQuestion-style answering: `options` as strings or `{label, description, preview}`,
  `header`, `multi_select`, notes, and `questions` for up to 4 grouped questions with drafts.
- `❓N` tab-label prefix with one shared config file, recorded marks (never stacks), and
  `sync-tokens --remove-prefixes` for uninstalling.
- Batched delivery, one message per pane; 0.1.x-compatible database schema.

## 0.1.1

- The Claude plugin runs from its installed copy with the environment in `${CLAUDE_PLUGIN_DATA}`.

## 0.1.0

- Initial proof of concept: SQLite inbox, MCP server (`ask_operator`, `check_answers`,
  `list_my_questions`, `withdraw_question`), Textual popup, herdr plugin manifest, Claude Code plugin
  with SessionStart/SubagentStart hooks and the `operator-questions` skill.
