// Captured on Claude Code 2.1.301, not invented: what an isolated session handed a probe mod
// (prompt.submit inputs, UserMessage render props, classic.PostToolBatch/Stop results) while
// herdr-inbox-shaped hooks delivered answers (PostToolBatch and Stop additionalContext, an
// asyncRewake exit 2) and answers were typed and pasted. How it was captured: the PR that added it.

export const typedSubmit = {
  "site": "prompt.submit",
  "text": "[inbox] Q16 answered — Typed one?: sure",
  "origin": {
    "kind": "composer"
  }
} as const

export const typedRow = {
  "site": "ui.render",
  "component": "UserMessage",
  "surface": "terminal",
  "requestId": "670b0b37-2b76-4775-b450-8d85c6213ec0",
  "props": {
    "text": "[inbox] Q16 answered — Typed one?: sure",
    "origin": {
      "kind": "composer"
    },
    "isExpanded": false
  }
} as const

export const pastedSubmit = {
  "site": "prompt.submit",
  "text": "\n\n<pasted_content id=\"5e9e\">\n[inbox] Q17 answered — Long one?: a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer \n</pasted_content id=\"5e9e\">\n",
  "origin": {
    "kind": "composer"
  }
} as const

export const pastedRow = {
  "site": "ui.render",
  "component": "UserMessage",
  "surface": "terminal",
  "requestId": "a0ba1b11-ed61-466e-ae86-bf2a6f09fc17",
  "props": {
    "text": "\n\n<pasted_content id=\"5e9e\">\n[inbox] Q17 answered — Long one?: a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer a long answer \n</pasted_content id=\"5e9e\">\n",
    "origin": {
      "kind": "composer"
    },
    "isExpanded": false
  }
} as const

export const rewakeSubmit = {
  "site": "prompt.submit",
  "text": "<task-notification>\n<summary>Stop hook feedback</summary>\n</task-notification>\n<system-reminder>\nStop hook blocking error from command \"Stop\": [inbox] Q15 answered — Merge now?: yes, after CI\n\n</system-reminder>",
  "origin": {
    "kind": "task-notification"
  }
} as const

export const rewakeRow = {
  "site": "ui.render",
  "component": "UserMessage",
  "surface": "terminal",
  "requestId": "b7f24baa-9957-4af9-af63-3794218fdf31",
  "props": {
    "text": "Stop hook feedback",
    "origin": {
      "kind": "task-notification"
    },
    "isExpanded": false,
    "task": {}
  }
} as const

export const postToolBatch = {
  "site": "classic.PostToolBatch",
  "result": {
    "additionalContext": [
      "[inbox] Q12 answered — Which DB?: Postgres"
    ]
  }
} as const

export const stop = {
  "site": "classic.Stop",
  "result": {
    "additionalContext": [
      "[inbox] answered — Q13 Ship today?: yes · Q14 Which region?: eu-west-1"
    ]
  }
} as const
