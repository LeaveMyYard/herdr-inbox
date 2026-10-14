---
name: operator-questions
description: How to ask the human operator a question without blocking, through the herdr-inbox ask_operator tool. Use when you need a decision, missing information or a preference from the operator and could keep working meanwhile, when you get an `[inbox]` answer message, after compaction or resume, or when briefing a subagent that may need to ask the operator something.
---

# Asking the operator through the inbox

The operator runs several agents at once. A question asked in chat is easy to miss, and a
question repeated every few turns is noise. The inbox gives each question one place: you file
it once with `ask_operator`, get a receipt (`Q12`), and keep working. The operator answers from
a popup; the answer arrives in your conversation as a message starting with
`[inbox] Q12 answered — …`. Answers the operator submits together arrive in one message:
`[inbox] answered — Q12 <question>: <answer> · Q13 <question>: <answer>`.

## When to use it

- Use it for anything you would otherwise ask the operator in chat **while you can keep
  working**: a choice between approaches, a missing value, a preference, a go/no-go on a
  later step.
- Do **not** use it for blocking or destructive confirmations (deleting data, deploying to
  production, force-pushing, spending money). Those still go through the normal flow: ask in
  chat and wait for an explicit go.
- If nothing useful can happen until the answer arrives, say so in chat once after filing the
  question, then stop instead of polling.

## Writing the question

The operator reads it in a list next to questions from other agents, without your chat
history. Make it self-contained:

- Name the repo, PR, branch, file or environment it is about.
- State what you need decided and what you will do with each answer.
- Put discrete choices in `options`, like AskUserQuestion: plain strings or
  `{"label": "squash", "description": "one commit on main"}`. The operator picks with the
  arrow or number keys, can attach a note to a choice ("Postgres (note: keep the old schema)"),
  or type a free answer instead. Up to 9 options.
- `header` is a short chip above the question (max 12 chars, e.g. "Merge mode").
- An option may carry a `preview` (plain monospace text such as a code snippet or config
  diff); the operator sees it beside the options while that option is highlighted.
- `multi_select: true` when several options may be picked; the answer lists them.
- Several related questions? Pass `questions` (up to 4), each
  `{"question", "header", "options", "multi_select"}`, instead of `question`. Each gets its own
  Q id; the operator answers them in one view and usually submits them together.
- Put long supporting detail (log excerpts, the tradeoff) in `context`, not in `question`.
- Set `asked_by` when you are a subagent or one of several roles in the pane, e.g.
  `"reviewer PR 42"`, so the operator knows who is asking.

Good: `question="api#42: merge with a merge commit or squash? The branch carries history from #40."`,
`options=["merge commit", "squash"]`.

Bad: `question="Which one should I use?"`

## After asking

- Do not repeat the question in chat and do not call `ask_operator` again for it. A repeat
  only returns the existing receipt (`already_open`) and bumps a repeat counter the operator
  sees.
- Mention the receipt once if useful ("asked the operator as Q12"), then continue with work
  that does not depend on the answer.
- When an `[inbox]` message with your Q id arrives, act on that answer.
- If you work it out yourself first, call `withdraw_question(id, reason)` so it leaves the
  operator's inbox.

## After compaction, /clear or resume

Call `list_my_questions` before asking anything new: it lists your open and answered
questions. `check_answers` returns answers you have not seen yet (for example ones delivered
before a compaction) and marks them seen.

## Subagents

Subagents do not see this guidance unless you pass it on. When you brief a subagent that may
need the operator, include: "If you need a decision from the operator and can keep working, call
`ask_operator` once with a self-contained question and `asked_by` set to your role; do not
wait for the answer; report the Q-number back." Answers are delivered to the main session's
pane, so you (the main agent) act on them.
