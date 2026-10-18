//! The inbox rules the SessionStart and SubagentStart hooks inject (Python
//! `hook.SESSION_RULES`, `hook.SUBAGENT_RULES`), word for word.

/// What every session starts with: how to use the inbox.
pub const SESSION_RULES: &str = r#"<herdr-inbox>
You have a non-blocking question inbox to the human operator: the `ask_operator` MCP tool (server herdr-inbox).
- Whenever you would ask the operator something in chat and can keep working meanwhile, call ask_operator instead. Make the question self-contained (the operator reads it without this chat). Like AskUserQuestion: choices in `options` (strings or `{label, description}`), a short `header`, `multi_select`, or up to 4 related questions at once via `questions` (then give the call a short `title`).
- Prefer it over AskUserQuestion, which stops you at a prompt the operator may not be watching.
- To flag something for the operator to check (no answer needed), call alert_operator instead of writing it in chat.
- Ask once. Never restate the question in chat and never call ask_operator again for it. Continue with work that does not depend on the answer.
- Answers arrive later as `[inbox]` messages naming the Q ids (several can come in one message); act on them then. If one seems overdue, call check_answers.
- After compaction or resume, call list_my_questions before asking anything new; check_answers returns answers you may have missed. If you resolve a question yourself, call withdraw_question.
- Blocking or destructive confirmations (anything you must not do without an explicit go) still go through the normal flow: ask in chat and wait.
- When you brief a subagent, pass this rule on: questions for the operator go through ask_operator, once, with `asked_by` set to its role, and it keeps working.
Details: skill herdr-inbox:operator-questions.
</herdr-inbox>"#;

/// What a subagent starts with.
pub const SUBAGENT_RULES: &str = r#"<herdr-inbox>
If you need a decision from the human operator and can keep working without it, call the `ask_operator` MCP tool once with a self-contained question and `asked_by` set to your role, then continue. Do not wait for the answer and do not repeat the question in your report beyond its Q-number; the answer is delivered to the main session. Blocking or destructive confirmations go back to your caller instead. Something the operator should check, with no answer needed, goes through `alert_operator` (title, message, severity; `asked_by` your role).
</herdr-inbox>"#;

/// The heading of a resumed or compacted session's open questions.
pub const PENDING_HEADING: &str = "\nYour questions already with the operator (do not ask them again; call check_answers for answers you have not seen):\n";

#[cfg(test)]
mod tests;
