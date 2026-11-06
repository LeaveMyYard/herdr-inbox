# herdr-app commands v1

How the herdr-app mods act on the inbox. A mod writes nothing itself: no `writeTextFile`, no SQLite. It
runs `herdr-inbox app <verb> --json` with `exec`. What it reads is the [feed](app-feed-v1.md). The
examples are in `tests/fixtures/app-commands/commands.json`.

**Status: nothing here is built yet.** B4 builds these verbs. `retry` needs B2's counter column.

## Running a command

There is no private copy of the binary. Every command a mod runs goes through the plugin's launcher, here
written `herdr-inbox`:

```
sh <root>/scripts/launch cli app <verb> ... --json
```

`<root>` is the inbox plugin's root. The [install record](app-feed-v1.md#installing) holds it, and a mod resolves
it again at `init`, because an update can move it and a herdr-app-only machine has no herdr to ask:

- **The operator's own git checkout** stays where it is: a pull updates it. A mod uses the recorded root or none,
  so an app home installed from a checkout never reaches another install.
- **Anything else** moves on an update: a herdr install is a new generation each time, with the old one deleted,
  and a Claude Code copy changes with the version. An old copy can stay on disk with an `engine` a rollback never
  reaches. So a mod takes the first of these that holds this plugin's `herdr-plugin.toml` and `scripts/launch`: `plugin_root`
  from `herdr plugin list --json`, which needs no server but waits on herdr's plugin lock, then the user-scope
  `installPath` in Claude Code's `installed_plugins.json`, then the recorded root.

With no such root the overlay says the inbox is unavailable. B4 adds `app` and `app-feed` to the launcher's `RUST_CLI`, and `tests/launch/cli.rs` holds that
list to the binary. So the `engine` file and the launcher's contract check apply to every verb, every
writer start and every sender a verb starts.

After a rollback to Python, `app` reaches the Python CLI, which has no such command and exits 2. A binary too
old to have `app` prints something else. Either way the `--help` probe below fails, and the overlay says the
inbox is unavailable. It never types with a binary the rollback meant to stop.

`herdr-inbox app --help` touches nothing, exits 0 and prints `herdr-inbox app v1` as its first line.

## Rules

- **Each verb is one short transaction.** The token check and the write are in the same `BEGIN
  IMMEDIATE`. The popup's `submit` loops over separate transactions; a verb must not, or the check would
  say nothing about the write.
- **The verb's own process never types.** It calls neither `herdr agent prompt` nor `hc send`. See
  [Delivery](#delivery).
- **The same writes as the popup, inside that transaction.** `set_draft`, `write_answer` (not
  `set_answer_text`, which opens its own transaction), `set_status` and `hide_failed`. The first three are
  compare-and-sets on `status = 'open'`. `hide_failed` has none, and alerts have their own, so for those
  the [verb's state check](#the-state-a-verb-checks) is the guard. With the popup and an overlay open on one
  group, the second one gets `stale` with `closed: "answered"`.
- **A budget of 5 s** for the whole command, well inside `exec`'s 30 s. The database wait is bounded
  inside it, and running out is `busy`, never a hang.
- **No answer text in output.** `message` and stderr never echo `--detail`.

## Invocation

| Verb | Arguments | What it does |
|---|---|---|
| `answer` | `--entry group:G --token T --question Q --detail JSON` | Answers the group's one open question at once, as the popup does for a lone question. On a group with more open questions it is `invalid`. |
| `answer` | `--entry group:G --token T --question Q [--question Q]...` | Submits the stored drafts of these questions, as the popup's Submit tab does. |
| `draft` | `--entry group:G --token T --question Q --detail JSON` | Stores an unsent answer. |
| `draft` | `--entry group:G --token T --question Q --clear` | Clears it. |
| `dismiss` | `--entry group:G --token T [--question Q]` | Closes that question, or without `--question` all the group's open questions. The mod asks first. |
| `retry` | `--entry failed:Q --token T` | Asks for another delivery attempt, see [below](#retry-and-force). There is no `--force`. |
| `stop-showing` | `--entry failed:Q --token T` | Hides the ⚠ row. The agent can still fetch the answer with `check_answers`. |
| `ack` | `--entry alert:A --token T` | Acknowledges the alert. |
| `show` | `--question Q` | Reads one question in full, see [below](#showing-one-question-in-full). Writes nothing. |

Every verb also takes `--json`, which is required. Arguments may come in any order, and each except
`--question` may come once.

`--entry` and `--token` are the row's `key` and `token` from the feed. `G`, `Q` and `A` are a group, question
and alert id: canonical decimal in [1, 9007199254740991], with no sign, prefix, leading zero, exponent or
fraction. A `--question` must be an open question of that group, else the verb is `invalid`.

`--detail` is a [Detail](app-feed-v1.md#detail) as one JSON argument of at most 64 KiB. The verb normalises
it as the popup does (`Detail::make`): notes only for picked labels, stripped, empty ones dropped; blank
text is none. It also puts `choices` in option order, which `Detail::make` does not do. It is `invalid` when
a choice is not one of the question's labels, is given twice, or when a question without `multi_select` gets
more than one. A detail that normalises to nothing is `invalid` for `answer`, and clears the draft for
`draft`. Real data can hold two options with one label (`normalize_options` does not dedupe); a label then
means the first such option, as in the popup.

`exec` has no stdin and the mod writes no files, so the answer is on the command line while the verb runs,
for up to the 5 s budget. On a default system any local user can read that from the process list, although
the feed is 0600. The sender's arguments hold ids only.

## Result

One JSON object on stdout. Empty stdout never means success.

| Exit | Stdout | Meaning |
|---|---|---|
| 0 | Success | It was done. |
| 1 | Refusal | Nothing was changed. |
| 2 | Empty | Bad arguments: usage on stderr, no I/O. That covers anything wrong without reading the database: a malformed `--entry` or id, a `--detail` that is no JSON object or is over 64 KiB. |

Any other exit, or a stdout that is not one such object, is a failure of unknown effect: the mod says so,
reloads the feed and does not repeat the verb by itself.

### Success

| Field | Type | Meaning |
|---|---|---|
| `v` | integer | Exactly 1. |
| `ok` | bool | True. |
| `verb` | string | The verb. |
| `entry` | string | The `--entry`. |
| `token` | ?string | The row's token now. Null exactly when the row is gone. |
| `done` | [integer] | The questions, or the alert, the verb changed. |
| `skipped` | [integer] | For `answer`: the named questions with no draft, as the popup's Submit skips them. Otherwise empty. |
| `delivery` | ?string | For `answer` and `retry`, see [Delivery](#delivery): `queued` or `held`. Null when nothing waits to be delivered. Never a claim that it arrived. |

The mod uses the `token` of its last result until a newer feed arrives. Without that, the next `draft`
on the same group would be refused against the mod's own previous write.

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

### Showing one question in full

`app show --question Q --json` is read-only: no transaction, no token, no write. It is the way out for a
question the feed [cut or left out](app-feed-v1.md#an-incomplete-feed). Its result has these fields, and a
refusal is as above: `stale` with `closed` when the question is not open.

| Field | Type | Meaning |
|---|---|---|
| `v` | integer | Exactly 1. |
| `ok` | bool | True. |
| `verb` | string | `show`. |
| `entry` | string | The row's `key`. |
| `row` | Entry | The `group` entry as the feed would publish it, but with no text cut and every `cut` false. Its `token` is the published one, computed on the cut text, so the other verbs take it. |

For a terminal, the same command without `--json` prints the question, its context and its options as plain
text. A mod that names it prints the real launcher command, as `herdr-inbox` is not on `PATH`. B4 builds it.

### The state a verb checks

Inside the transaction, in this order:

1. **The row exists for verbs.** A `group` has open questions. A `failed` row is answered, not delivered, not
   fetched and not hidden; a live claim and the 60 s wait do not matter here, though they keep the row out
   of the feed. An `alert` is open. Otherwise `stale`, with `closed`.
2. **For `retry`, no live claim.** Otherwise `already_sending`, before the token, so that a shown row the worker
   has just claimed reads as what it is. The worker claims pending rows on every pass, so this is common.
3. **The token,** computed from that row. Otherwise `stale`.

`stop-showing` does not look at claims, as the popup's does not: the row just stops showing, and a send in
progress goes on. A `retry` on a row that became `agent_gone` after it was shown is done: the popup allows
that too, and the sender checks the conversation before it types.

### Repeating a verb

A hot reload during an `exec`, or a repeat after a lost result, can do no harm, and no request id is needed:

- `answer`, `dismiss`, `stop-showing` and `ack` closed the row or changed its token, so a repeat is `stale`.
- `retry` raises the row's `retries` in its transaction (`question_ext.retry_count`, from B2's revision bump).
  The token covers that, so a repeat is `stale` and cannot force a second send. Only an operator's new
  key press, with the token from the result or a newer feed, retries again.
- `draft` with the same content, `--clear` on no draft, and a Submit with nothing to send change nothing.
  They succeed again and have no effect.

The popup's own Retry does not raise `retries`, so an overlay can still retry after the popup did. That takes
two deliberate presses, in two UIs.

## Delivery

`answer` and `retry` do this, in this order:

1. **Commit** the transaction, and `retry`'s counter with it. The sender skips a row that is not answered yet.
2. **Ensure the worker** of the row's stored server (`ensure_worker(Some(server))`), first, so that a sender
   lost after this point is retried. This holds even if step 3 stops the sender: on a dead server the worker
   exits in about 10 s and uses no attempt, and it is what brings the answer once the host is back.
3. **Decide whether a sender can go,** by the table below.
4. **Start the detached sender,** the popup's: `deliver <ids> [--force]`, with `HERDR_SOCKET_PATH` set to the
   stored server. The result is `delivery: queued`.

The verb's own process never types and never waits for the sender. The sender works under the delivery
claim, like every other send.

| Row | Step 3 | Route if `held` |
|---|---|---|
| TUI, stored server | Ask that server, within the budget. Only `HerdrError::NotRunning` proves it gone, as for the worker (`server_gone`), and that gives `held`. Any other failure is no proof, and the sender goes. | That server's worker, when it is back. |
| TUI, no stored server (asked before 0.6.1), with a session | The same, on the verb's default socket. `deliver` treats the row as local, and its `conversation_changed` guard checks the session. | The worker's own proof of the conversation. |
| TUI, no stored server, no session | Always `held`, for `retry` too. The recipient rule beats retry: that guard is false with no session, so a forced send could reach another agent in a pane with the same id. | `check_answers` only. |
| No pane and no host record (`unknown`) | Always `held`. | `check_answers` only: the worker's listing never takes such a row. |
| Core | Ask `hc`. Only a proven absence of herdr-core gives `held`. | The core worker, when herdr-core is back. |

A `held` result says that no sender started, for one of these reasons. A sender started there would burn a counted
attempt for each press, towards marking the agent gone.

**The server travels with the answer.** A worker serves only its own server's rows, and `deliver` leaves an
answer of another server to that server's worker, dropping any `--force` silently. `exec` has no env option, so
the verb's environment names the default socket. B4 therefore adds a server parameter to `start_sender`,
passed to `spawn_env` as `ensure` does.

**A core row's server is `herdr-core`.** B2 stores that reserved word in `question_ext.herdr_server`, not a
socket and not NULL (`CORE_SERVER`). It is no herdr server's id, so every herdr worker, and Python's `deliver`,
treat the row as another server's. B3's worker is keyed on the same word: one core worker per
machine, with the lock `worker-<sha1("herdr-core")[:12]>.lock`, alive while herdr-core is up. So step 2 is
`ensure_worker(Some("herdr-core"))`. Step 4 sets no socket variable: Rust's `deliver` takes `herdr-core` rows
as its own, whichever herdr server it runs under, and asks for that worker first.

| `delivery` | Meaning |
|---|---|
| `queued` | A sender started, or failed to start and left the answer to the worker just ensured. |
| `held` | The answer is recorded and no sender started. The table above says what can still bring it. |

A `retry` that comes out `held` is still done: its counter moved, and the mod says that nothing could be sent.
That is what stops a replay forcing a send once the server is back.

Marks are cosmetic. For a TUI question a verb may refresh the pane's `$inbox_q` and `❓N` after its commit.
That is bounded by the budget and a failure never fails the verb.

From here the mod reads progress from the feed: the row leaves when the answer is delivered or fetched. See
[`failed`](app-feed-v1.md#failed) for when a row appears and what it means.

### Retry and force

`retry` is forced on TUI and never forced on core.

| Row | Sender | Effect |
|---|---|---|
| `herdr-tui` | `deliver <id> --force` | The popup's Retry / Send now. It goes past the watcher, hook, requeue and panel waits, the busy grace and the focus rule. It does not go past `blocked`, an unsent draft in the input box, or the conversation checks. |
| `herdr-core` | `deliver <id>` | Another attempt under the normal rules. |

**The rule lives in the core send path, not in the verb.** A core row reaches every popup too, because
`failed_deliveries` has no pane rule. So `Mode.force` has no effect on a core row, and the `hc send`
adapter has no way to express force: it always passes `--if idle`. That covers the Rust popup from P6-E. The
verb has no `--force` argument and takes the host from the database inside its transaction, never from the
mod. It starts core senders without `--force` so that its intent is plain, but it is not the safeguard.

The popup is Python until P6-E whatever `engine` says, and its Retry runs Python's `deliver`, which never
reaches that code. That is safe because Python has no way to type to a core row, and because of
`herdr-core`: it is another server's row there, so Python claims nothing and writes nothing. Without that word
Python would record `no pane: answer available through check_answers` and count an attempt, on an answer given
in the popup as well as on Retry. The row `python-deliver-leaves-a-core-row` of `tests/fixtures/decisions-core.json`
pins this path until P6-E: Python `deliver` on a core row leaves it deliverable. Python still asks for a
`herdr-core` worker, and the core worker lists the row, so under `engine` rust the answer goes out within
the 60 s of a never-attempted row, or at once by the watcher.

The mod does not call `retry` "Send now" on a core row.

## Going to an agent

This is no verb, because it writes nothing.

| `asker.host` | The mod runs |
|---|---|
| `herdr-core` | The app command `agent.focus {terminal}`. |
| `herdr-tui` | `herdr-inbox open --question Q<id>`, which opens the herdr TUI popup on that question. It does not focus the pane. Exit 1: no herdr server, or herdr refused. Exit 3: a popup is open already. |
| `unknown` | Nothing to go to. |

A remote machine's agents are opened from herdr's machine view.
