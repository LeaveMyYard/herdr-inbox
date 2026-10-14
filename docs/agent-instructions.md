# Agent instructions (manual install only)

The Claude Code plugin injects these rules into every session through its SessionStart hook,
so you do not need this file when the plugin is installed. If you registered only the MCP
server (`claude mcp add ...`), paste the block below into your `CLAUDE.md`.

```markdown
## Questions for the operator

- Questions for the operator go through the `ask_operator` MCP tool (herdr-inbox), not chat,
  whenever you can keep working meanwhile. Make the question self-contained (the operator
  reads it without this chat) and put choices in `options`.
- Ask once. Never restate the question in chat and never call `ask_operator` again for it.
  Continue with work that does not depend on the answer.
- Answers arrive as messages starting with `[inbox] Q<n> answered`; act on them then.
- After compaction or resume, call `list_my_questions` before asking anything new;
  `check_answers` returns answers you may have missed. If you resolve a question yourself,
  call `withdraw_question`.
- Blocking or destructive confirmations still go through the normal flow: ask in chat and wait.
- When briefing a subagent, pass the rule on: `ask_operator` once, with `asked_by` set to its
  role, and keep working.
```
