# Changelog

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
  it appears. The `+N new` marker is gone (arrivals are list rows).

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
