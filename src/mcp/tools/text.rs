//! What agents read about the server and its tools (Python `server.INSTRUCTIONS`,
//! `ASK_DESCRIPTION`, `ALERT_DESCRIPTION` and the tools' docstrings), word for word:
//! agent behaviour depends on this text, so `tests/mcp_goldens.rs` holds it to the
//! Python server's `tools/list` (`tests/fixtures/mcp/legacy.jsonl`).

/// What Claude Code adds to the agent's context about this server.
pub const INSTRUCTIONS: &str = "Non-blocking question inbox for the human operator. Use ask_operator instead of asking the \
operator in chat whenever you can keep working. Ask once; the answer arrives later as a \
`[inbox]` message naming the Q id. After compaction call list_my_questions \
before asking anything. To flag something the operator should check, with no answer \
needed, use alert_operator.";

/// `ask_operator`.
pub const ASK: &str = r#"Ask the human operator a question WITHOUT blocking. Returns immediately with a receipt.

Use this instead of asking in chat whenever work can continue without the answer.
The operator answers asynchronously; the answer will arrive in this conversation as a
message starting with `[inbox]` that names the question id (Q<n>). After calling it, continue
with work that does not depend on the answer. Do not repeat the question in chat and do not
call ask_operator for it again: a repeat is detected and only returns the existing receipt.

Keep the question self-contained: the operator reads it without your chat history, so name
the repo/PR/file and what you need decided. The operator answers in a picker like
AskUserQuestion's:
- `options`: up to 9 choices, each a plain string or `{"label", "description"}` (label short,
  description says what picking it means). The operator can always type a free answer instead
  and attach a note to a choice.
- `header`: a short chip shown above the question (max 12 chars), e.g. "DB choice".
- `multi_select`: true when several options may be picked together.
- `preview` (per option, optional): text shown beside the options while that option is
  highlighted (a code snippet, a config diff, an ASCII mock); monospace, no markup.
- `title`: a short name for the whole call (≤60 chars), e.g. "Worker rollout settings". The
  operator's inbox lists calls by this name, so ALWAYS set it when you pass `questions`;
  optional for a single question (its header or text is used instead).
- `questions`: instead of `question`, up to 4 related questions in one call, each
  `{"question", "header"?, "options"?, "multi_select"?}`. The operator answers them together;
  each gets its own Q id.
- `context`: supporting detail; `asked_by`: who is asking (e.g. "reviewer PR 42") when you are
  a subagent or one of several roles.
Each answer is one line: the picked label(s), notes in parentheses, or the free text. Answers
the operator submits together arrive in one `[inbox]` message."#;

/// `alert_operator`.
pub const ALERT: &str = r#"Alert the human operator WITHOUT blocking and without expecting an answer: "please
check X", "the deploy needs your eyes", "I hit something you should know about".

Use it instead of writing the warning into chat (the operator may not be looking at this pane)
and instead of stopping to wait. It shows in the operator's inbox popup and as a herdr
notification, and counts in this tab's ❓ badge, until the operator acknowledges it.
Nothing is typed back; check_answers reports acknowledged alerts. Use ask_operator instead
when you need a decision or information. `severity`: info | warning | urgent. Raise one alert
per issue; call withdraw_alert if it resolves itself."#;

/// `withdraw_alert` (a docstring).
pub const WITHDRAW_ALERT: &str =
    "Withdraw one of your open alerts because the issue resolved itself.";

/// `check_answers` (a docstring: its continuation lines keep their indentation).
pub const CHECK_ANSWERS: &str = "Return operator answers to your questions that you have not seen yet, and mark them seen.\n\n    \
Answers are normally delivered as `[inbox]` messages naming the Q ids; use this as a fallback\n    \
(e.g. after compaction or if a delivery was missed).";

/// `list_my_questions` (a docstring).
pub const LIST_MY_QUESTIONS: &str = "List the questions this session has asked the operator, with their status and answers.\n\n    \
Call this after compaction or resume BEFORE asking anything, so you do not re-ask a\n    \
question that is already open or answered.";

/// `withdraw_question` (a docstring).
pub const WITHDRAW_QUESTION: &str = "Withdraw one of your open questions because you resolved it yourself.\n\n    \
It is removed from the operator's inbox.";
