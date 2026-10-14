# Changelog

## 0.5.1

- **Fix: answers could be lost when the asking agent was busy.** `herdr agent prompt` succeeding
  only means the keys reached the pane; in live use answers typed while the agent was mid-turn
  never reached the conversation, yet were marked delivered and seen. Now:
  - an answer is typed only when herdr reports the agent `idle`/`done`; while it works the answer
    is *pending delivery* (shown as `⏳` in the popup) and the asking session's retry loop (every
    10 s) delivers it at the next idle;
  - a delivery counts only once the text shows up in the session's transcript (a `user` record,
    or a queued `queue-operation`/`queued_command`) within 15 s; otherwise it is retried;
  - "seen" means the agent fetched the answer (`check_answers`/`list_my_questions`) or the
    delivery was verified, never just "sent"; the receipt and the SessionStart text point to
    `check_answers` as the fallback.
- Recovery: recent answers that were marked seen without verification become unseen again once,
  so `check_answers` returns them; `inbox redeliver <ids>` delivers answers again.
- The SessionStart hook records the session's transcript path for verification.

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
