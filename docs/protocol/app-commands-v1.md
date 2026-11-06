# herdr-app commands v1

How the herdr-app mods act on the inbox. A mod writes nothing itself: no `writeTextFile`, no SQLite. It
runs `herdr-inbox app <verb> --json` with `exec`. What it reads is the [feed](app-feed-v1.md). The
examples are in `tests/fixtures/app-commands/commands.json`.

**Status: nothing here is built yet.** B4 builds these verbs.

## Rules

- **Each verb is one short transaction.** The token check and the write are in the same `BEGIN
  IMMEDIATE`. The popup's `submit` loops over separate transactions; a verb must not, or the check would
  say nothing about the write.
- **The verb's own process never types.** It calls neither `herdr agent prompt` nor `hc send`. It records
  and returns. See [Delivery](#delivery).
- **The same writes as the popup.** `set_draft`, `set_answer_text`, `set_status`, `hide_failed` and
  `acknowledge_alert`, under the same compare-and-set on `status = 'open'`. With the popup and an overlay
  open on one group, the second one gets `stale` with `closed: "answered"`.
- **A budget of 5 s** for the whole command, well inside `exec`'s 30 s. The database wait is bounded
  inside it, and running out is `busy`, never a hang.

`exec` has no stdin, so everything goes in arguments, the answer too. See `--detail` below.

## Invocation

`herdr-inbox app --help` touches nothing, exits 0 and prints `herdr-inbox app v1` as its first line.

| Verb | Arguments | What it does |
|---|---|---|
| `answer` | `--entry group:G --token T --question Q --detail JSON` | Answers the group's one open question at once, as the popup does for a lone question. |
| `answer` | `--entry group:G --token T --question Q [--question Q]...` | Submits the stored drafts of these questions, as the popup's Submit tab does. |
| `draft` | `--entry group:G --token T --question Q --detail JSON` | Stores an unsent answer. |
| `draft` | `--entry group:G --token T --question Q --clear` | Clears it. |
| `dismiss` | `--entry group:G --token T [--question Q]` | Closes that question, or without `--question` all the group's open questions. The mod asks first. |
| `retry` | `--entry failed:Q --token T` | Asks for another delivery attempt. |
| `stop-showing` | `--entry failed:Q --token T` | Hides the ⚠ row. The agent can still fetch the answer with `check_answers`. |
| `ack` | `--entry alert:A --token T` | Acknowledges the alert. |

Every verb also takes `--json`, which is required. Arguments may come in any order, and each except
`--question` may come once.

`--entry` and `--token` are the row's `key` and `token` from the feed. `Q` and `A` are canonical decimal
ids in [1, 9007199254740991]: no sign, prefix, leading zero, exponent or fraction.

`--detail` is a [Detail](app-feed-v1.md#detail) as one JSON argument of at most 64 KiB. The verb
normalises it as the popup does (`Detail::make`): notes only for picked labels, stripped, empty ones
dropped; blank text is none. It is `invalid` when a choice is not one of the question's labels, or when
a question without `multi_select` gets more than one choice. A detail that normalises to nothing is
`invalid` for `answer`, and clears the draft for `draft`.

The answer is on the command line for as long as the verb runs, so a process listing can show it. That
is the price of `exec` having no stdin and of the mod writing no files.

## Result

One JSON object on stdout. Empty stdout never means success.

| Exit | Stdout | Meaning |
|---|---|---|
| 0 | Success | It was done. |
| 1 | Refusal | Nothing was changed. |
| 2 | Empty | Bad arguments. Usage on stderr, no I/O. |

Any other exit, or a stdout that is not one such object, is a failure of unknown effect: the mod says so,
reloads the feed and does not repeat the verb by itself.

### Success

| Field | Type | Meaning |
|---|---|---|
| `v` | integer | Exactly 1. |
| `ok` | bool | True. |
| `verb` | string | The verb. |
| `entry` | string | The `--entry`. |
| `token` | ?string | The row's token now. Null when the row is gone. |
| `done` | [integer] | The questions, or the alert, the verb changed. |
| `skipped` | [integer] | For `answer`: the named questions with no draft, as the popup's Submit skips them. Otherwise empty. |
| `delivery` | ?string | `queued` when an answer now waits for delivery, else null. Never a claim that it arrived. |

The mod uses the `token` of its last result until a newer feed arrives. Without that, the next `draft`
on the same group would be refused against the mod's own previous write. It also holds for a `draft`
that clears.

### Refusal

| Field | Type | Meaning |
|---|---|---|
| `v` | integer | Exactly 1. |
| `ok` | bool | False. |
| `verb` | string | The verb. |
| `entry` | string | The `--entry`. |
| `code` | string | See below. An unknown code reads as `error`. |
| `message` | string | Plain text the mod may show. |
| `closed` | ?string | For `stale`, when the row or question has closed: `answered`, `dismissed`, `withdrawn`, `delivered`, `fetched`, `hidden`, `acked`, `cleared` or `removed`. Otherwise null. |

| `code` | Meaning | The mod |
|---|---|---|
| `stale` | The token does not match, or the row no longer exists. | Says "this group changed, look again", reloads, and waits for the operator's next key. It never retries with a newer token. |
| `already_sending` | `retry` on an answer a process holds a live claim on. | Says so and reloads. |
| `invalid` | The detail or `--question` does not fit the row. | Shows `message`. A mod bug if it happens. |
| `read_only` | A remote row, or a verb the row's `actions` do not list. | Shows `message`. |
| `busy` | The database stayed locked for the budget. | Says "try again". The operator's input stays. |
| `not_migrated` | The database is not at this code's revision. The verb started a worker, which migrates it. | Says "try again in a few seconds". |
| `error` | Anything else. | Shows `message`. |

### Repeating a verb

A verb that was done and is run again is refused as `stale`, because its own write changed the token or
closed the row. It never writes twice. So a hot reload during an `exec`, or a repeat after a lost result,
can do no harm, and no request id is needed.

## Delivery

`answer` records. Then it makes sure of a worker for each answer's own herdr server, the one stored with
the question (`ensure_worker(Some(server))`), not the one the verb's environment happens to name. A
worker serves only its own server's rows, and it exits when idle, so the default socket's worker would
not reach another server's answer. A verb never waits for delivery.

Marks are cosmetic. For a TUI question a verb may refresh the pane's `$inbox_q` and `❓N` after its
commit. That is bounded by the budget and a failure never fails the verb.

From here the mod reads progress from the feed: the row leaves when the answer is delivered or fetched.
An answer that stays shows up as a `failed` row after 60 s, with its `delivery`.

**Not settled: what makes delivery start.** The contract above does not depend on the answer, but a real
delay does:

- `record` plus `worker --ensure` alone is slow. The worker's retry loop skips an answer nobody has
  attempted until it is 60 s old (`NEVER_ATTEMPTED_GRACE`). The popup does not wait for that: after
  recording, it starts a detached `herdr-inbox deliver`, and it types an answer for the pane it returns
  to itself.
- `retry` is the popup's "Send now": `deliver` with `--force`, past the busy grace, the idle gate and the
  focus rule. The worker's retries never force. And a core terminal must never get `hc send --force`,
  so `retry` cannot mean the same on both hosts.

So B4 needs either the verb to start the same detached sender as the popup, or the worker to learn to
take fresh answers and retries. Until that is decided, a mod promises nothing about timing, and shows
`queued` as "sending". In both cases the verb's process itself does not type, and every send is under
the delivery claim.

## Going to an agent

This is no verb, because it writes nothing.

| `asker.host` | The mod runs |
|---|---|
| `herdr-core` | The app command `agent.focus {terminal}`. |
| `herdr-tui` | `herdr-inbox open --question Q<id>`, which opens the herdr TUI popup on that question. It does not focus the pane. Exit 1: no herdr server, or herdr refused. Exit 3: a popup is open already. |
| `unknown` | Nothing to go to. |

A remote machine's agents are opened from herdr's machine view.
