# herdr-app feed v1

The contract between the Rust inbox and the herdr-app mods (`herdr-inbox`, `herdr-inbox-toast`). Rust
owns every decision and every write. A mod reads this one file and acts only through the verbs in
[app-commands v1](app-commands-v1.md). The fixtures are in `tests/fixtures/app-feed/` and
`tests/fixtures/app-commands/`.

**Status: nothing here is built yet.** This is the document the writer (B4) and the mods (B5, B6) are
built against. [What depends on what](#what-depends-on-what) says what holds today and what waits for
a later slice. [Not settled](#not-settled) lists what this version leaves open.

## Delivery

The writer publishes UTF-8 JSON at `<appHome>/data/herdr-inbox/feed.json`. `<appHome>` is
`$HERDR_APP_HOME`, otherwise `$HOME/.config/herdr-app`, as `appHome()` returns it.

It is a full snapshot, never a delta. Readers see complete files only: the writer fills a sibling
temporary file and renames it over the feed. The file is mode 0600 and its directory 0700. It holds
question text, which the TUI popup also shows, and mods are trusted code.

A mod reloads on `watchFolder(<appHome>/data/herdr-inbox)`, with a slow timer as the fallback, and
reads with `readTextFile`.

**The feed is at most 1 MiB.** `readTextFile` reads no more than 1 MiB (1048576 bytes), so the writer
never publishes a larger file. When the entries do not fit, it keeps them in order, drops whole entries
from the end, counts them in `omitted` and adds a line to `errors`. An entry is never cut short: its
token covers all of it, and an answer must not be given to text the operator only partly saw. An entry
that alone cannot fit is dropped the same way.

## Reading

Readers require a JSON object whose `v` is the number 1. A missing version, `true` or another version
rejects the file. Every field in the tables below is required, nullable fields and empty arrays
included. `?T` means T or null, `[T]` an array, and capitalised names refer to other tables.

| Type | Meaning |
|---|---|
| `integer` | A JSON integer within ±9007199254740991. |
| `number` | Finite POSIX seconds; a fraction is allowed. |
| `time` | A UTC time as the inbox stores it: `YYYY-MM-DDTHH:MM:SS+00:00`. |
| `string` | Plain text. Never markup, never a command. |

Unknown fields are ignored. Changing a field's type or meaning needs a new version. A vocabulary word
the reader does not know stays visible: it shows the row with the fallback the tables give and offers
no action on it.

On a missing, unreadable or unsupported file, or one with `ok: false`, the reader keeps its last good
feed, marks it stale and keeps its age. With no last feed it shows that the inbox is unavailable. A read
error never becomes an empty inbox.

A feed is stale when `generated_at` is missing, in the future, or more than 15 s old. That is a view
rule, not proof the writer died. A stale feed stays on screen. Its tokens still protect every action,
because a verb checks them against the database and not against the file.

## Feed

| Field | Type | Meaning |
|---|---|---|
| `v` | integer | Exactly 1. |
| `generated_at` | number | When this snapshot was read. |
| `ok` | bool | The local questions and alerts were read. False: `entries` is empty and says nothing about the inbox. |
| `errors` | [string] | What went wrong, as plain text. May be non-empty while `ok` is true: the entries then still hold every local row, and a remote machine's rows or the ones counted in `omitted` are what is missing. |
| `omitted` | integer | Entries dropped to stay under 1 MiB. 0 normally. |
| `new_since` | NewSince | Where arrivals stand. |
| `entries` | [Entry] | Every row the inbox shows, in the [order below](#order). |

### NewSince

The highest ids filed so far, open or not.

| Field | Type | Meaning |
|---|---|---|
| `question_id` | integer | The highest question id. 0 if there is none. |
| `alert_id` | integer | The highest alert id. 0 if there is none. |

The toast mod keeps the marks in memory:

- Its first good feed sets them, and nothing is announced. That is what keeps a restart or a hot reload
  from announcing old rows.
- From then on, it announces each `group` entry whose `group` is above `question_id`, and each `alert`
  entry whose `id` is above `alert_id`. Then it takes the new marks.

A file the reader missed loses nothing: the comparison is against ids, not against the previous file.
A group's id is its first question's id, and every `ask_operator` call that files anything starts a new
one, so an arrival always has a higher id. Remote machines' rows have no ids that compare, and v1 does
not announce them.

## Entries

Every entry has these fields, then those of its `kind`.

| Field | Type | Meaning |
|---|---|---|
| `key` | string | The row's name. The same spelling as the popup's transition golden (`tests/fixtures/popup-transitions.json`), and what a verb's `--entry` takes. |
| `kind` | string | See the table below. An unknown kind: show `key`, offer nothing. |
| `token` | ?string | The row's revision token, see [Tokens](#tokens). Null when `actions` is empty. |
| `actions` | [string] | The verbs this row takes now. |

| `kind` | `key` | The row | `actions` |
|---|---|---|---|
| `group` | `group:<group>` | One `ask_operator` call's open questions. | `answer`, `draft`, `dismiss` |
| `failed` | `failed:<question>` | An answer not delivered (⚠). | `retry`, `stop-showing` |
| `alert` | `alert:<id>` | An alert, raised by an agent or by the blocked scan. | `ack` |
| `remote_group` | `remote:<machine>\|<db>\|<group>` | A remote machine's open questions. | none in v1 |
| `remote_outbox` | `routbox:<request>` | An action queued for a remote machine, or one it failed or refused. | none in v1 |
| `remote_problem` | `rproblem:<machine>\|<question>` | A remote answer its machine cannot deliver. | none in v1 |
| `remote_alert` | `ralert:<machine>\|<id>` | A remote machine's open alert. | none in v1 |

The popup has one more row, `gone`. It is not in the feed, because it depends on when a popup opened;
see [What the reader derives](#what-the-reader-derives).

### Order

Alerts, then remote alerts, then groups by their first open question's id, then `failed` by question
id, then remote groups, outbox rows and problems. This is the popup's order without its focused pane
rule.

### `group`

| Field | Type | Meaning |
|---|---|---|
| `group` | integer | The group id. |
| `title` | ?string | The group's title, if the agent gave one. |
| `created_at` | time | When its first open question was asked. There is no age field: the reader works it out, so the file does not change every second. |
| `asker` | Asker | Who asked. One call has one asker. |
| `questions` | [Question] | The open questions, by id. Never empty. |

### Question

| Field | Type | Meaning |
|---|---|---|
| `id` | integer | Shown as `Q<id>`. |
| `header` | ?string | The chip, at most 12 characters. |
| `question` | string | The text. |
| `options` | [Option] | In order, at most 9. Empty: free text only. |
| `multi_select` | bool | Several options may be picked. |
| `context` | ?string | Its context lines. |
| `asked_by` | ?string | The agent's role. |
| `draft` | ?Detail | The operator's unsent answer. |

### Option

| Field | Type | Meaning |
|---|---|---|
| `label` | string | What the operator picks. |
| `description` | ?string | The dim line under it. |
| `preview` | ?string | The preview pane's text. |

### Detail

An answer in parts, draft or final. The verbs take the same object.

| Field | Type | Meaning |
|---|---|---|
| `choices` | [string] | Picked labels, in option order. |
| `notes` | object | A string per picked label that has a note. |
| `text` | ?string | Typed text. |

### Asker

| Field | Type | Meaning |
|---|---|---|
| `host` | string | `herdr-tui`, `herdr-core` or `unknown`, as in Panorama's feed. Any other word reads as `unknown`. |
| `pane` | ?string | The herdr pane. Set exactly when `host` is `herdr-tui`. |
| `terminal` | ?string | The core terminal address, such as `t3`. Set exactly when `host` is `herdr-core`. |
| `tab_label` | ?string | The tab's label when it asked, without the inbox's `❓N` prefix. Null for a core asker: the mod joins `terminal` with `terminals()`. |
| `cwd` | ?string | The agent's working directory. |

`unknown` is a row with neither a pane nor a host record. A current `ask_operator` never files one, but
old rows can hold it. It can be answered and nothing can be focused.

A core terminal has no `HERDR_PANE_ID`, and its question has `pane_id` NULL in the database. Space and
tab membership are not in the feed: herdr-app already knows them, through `terminals()` and `panes()`.

### `failed`

An answer the agent has not had. The rule is `Store::failed_deliveries`: answered, not delivered, not
fetched by `check_answers`, not hidden, and either with a delivery error or never attempted for 60 s.
An answer a process holds a live claim on is being sent, so it is no row at all.

| Field | Type | Meaning |
|---|---|---|
| `question_id` | integer | The question. |
| `group` | integer | Its group. |
| `header` | ?string | Its chip. |
| `question` | string | Its text. |
| `asked_by` | ?string | The agent's role. |
| `answer` | ?string | The answer as it will be sent. |
| `answered_at` | time | When the operator answered. |
| `asker` | Asker | Who asked. |
| `delivery` | Delivery | Where delivery stands. |

### Delivery

| Field | Type | Meaning |
|---|---|---|
| `state` | string | See below. An unknown word: show `reason`. |
| `reason` | ?string | The stored reason as plain text, its prefix removed. Null for `unattempted` and `claimed`. |
| `wait` | ?string | For `pending`, why it waits. Null otherwise. |
| `attempts` | integer | Failed attempts so far. |
| `pending_since` | ?time | Since when it waits for a busy agent. |
| `auto_retry` | bool | The worker of its host will try again by itself. False when the agent is gone, or when `no agent` / `conversation changed` / `not verified` has used up its tries. |

| `state` | Stored `delivery_error` | Meaning |
|---|---|---|
| `unattempted` | NULL | Nobody tried yet, for 60 s or more. |
| `claimed` | `sending:…`, older than 60 s | A process died while sending. The claim token is never published. |
| `pending` | `pending: …` | It waits for something. It is not a failed attempt. |
| `agent_gone` | `agent gone: …` | The agent is gone. `check_answers` can still return the answer. |
| `failed` | anything else | The last attempt failed. |

These are the state names of `tests/fixtures/decisions.json`.

The first row that fits decides:

| `wait` | Stored reason | The popup says |
|---|---|---|
| `busy_idle` | `BUSY_WAIT_IDLE` | It sends when the agent is idle. |
| `requeued` | starts `requeued:` | Not delivered earlier; it resends when the agent is idle. |
| `panel` | `PANEL` | The agent's task panel has focus. |
| `busy` | starts `busy:` | The agent is busy; it sends when the grace ends. Count it from `pending_since`. |
| `blocked` | starts `blocked:` | The agent is at a prompt or dialog. |
| `operator_here` | `waiting: the operator is in that pane` | It waits for the operator to leave that pane. |
| `other` | anything else | Show `reason`. |

The mod writes its own wording from these words and does not parse `reason`.

### `alert`

| Field | Type | Meaning |
|---|---|---|
| `id` | integer | Shown as `A<id>`. |
| `alert_kind` | string | `agent` or `blocked`. An unknown word reads as `agent`. |
| `severity` | string | `urgent`, `warning` or `info`. An unknown word reads as `warning`. |
| `title` | string | At most 120 characters. |
| `message` | ?string | At most 2000 characters. |
| `asked_by` | ?string | Who raised it. |
| `created_at` | time | When it was raised. |
| `asker` | Asker | Its agent. `cwd` is always null: alerts do not store one. |

### Remote rows

These come from the local mirror of the remote machines (`docs/REMOTE.md`). They are read-only in v1.
The operator acts on them in the TUI popup.

Machine:

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Its machine id. |
| `label` | ?string | Its name. Null shows the id. |
| `offline` | ?string | The offline note when the last pull failed. |

`remote_group`:

| Field | Type | Meaning |
|---|---|---|
| `machine` | Machine | Where it runs. |
| `title` | ?string | The group's title. |
| `created_at` | ?time | When its first question was asked. |
| `tab_label` | ?string | Its tab over there. |
| `questions` | [Question] | Its open questions. Every `draft` is null. |

`remote_outbox`:

| Field | Type | Meaning |
|---|---|---|
| `machine` | Machine | Where it goes. |
| `action` | string | `answer`, `dismiss`, `hide` or `ack_alert`. |
| `remote_id` | integer | The question, or for `ack_alert` the alert, over there. |
| `state` | string | `queued`, `rejected` or `failed`. |
| `result` | ?string | Why it failed or was refused. |
| `sending` | bool | Being sent right now. |
| `expired` | bool | Queued past its expiry. |
| `expires_at` | time | Its expiry. |

`remote_problem`:

| Field | Type | Meaning |
|---|---|---|
| `machine` | Machine | Where it ran. |
| `remote_id` | integer | The question over there. |
| `why` | string | Why it cannot be delivered. |
| `question` | string | Its text. |
| `answer` | ?string | The answer. |

`remote_alert`:

| Field | Type | Meaning |
|---|---|---|
| `machine` | Machine | Where it runs. |
| `id` | integer | The alert over there. |
| `alert_kind` | string | As in `alert`. |
| `severity` | string | As in `alert`. |
| `title` | string | Its title. |
| `message` | ?string | Its message. |
| `tab_label` | ?string | Its tab over there. |
| `asked_by` | ?string | Who raised it. |
| `created_at` | ?time | When it was raised. |

## Tokens

A token says "the row the operator is looking at". Every verb takes one, and a token that no longer
matches is refused: this is the JS counterpart of the popup's rule that no background event changes
what the next key does.

```
token = first 32 hex digits of sha256( canonical( entry without token, actions and delivery ) )
```

`canonical` is the entry's JSON with object keys sorted by code point at every level, no whitespace,
UTF-8, and non-ASCII characters written as themselves (Python's
`json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`).

A verb builds the entry from the database and computes this again, in the same transaction as its
write. So a token changes when an open question leaves a group, when any text, option or asker changes,
and when anyone's draft changes. It does not change when `delivery` moves, which happens all the time
behind a ⚠ row without changing what Retry means; the verb checks that state on its own.

A mod treats a token as an opaque string and only echoes it. Only Rust computes one.

## What the reader derives

- **The focused pane first.** The popup lists the focused pane's groups first. A mod may do the same
  with its own focus.
- **`gone`.** When the list opens, two or more `failed` entries with `state: agent_gone` become one
  summary row with the key `gone`, and that set stays fixed while the list is open. This depends on
  when a UI opened, so the model does it (P6-A's transition golden has the rule). It acts on the row
  with one `stop-showing` per entry.
- **Tabs.** One open question is answered at once. Several get a Submit tab.
- **Ages,** from `created_at` and the reader's clock.

Going to an agent is no verb. See [app-commands v1](app-commands-v1.md#going-to-an-agent).

## Runtime

### The writer

`herdr-inbox app-feed --help` starts nothing, touches no file, exits 0 and prints
`herdr-inbox app-feed v1` as its first line.

`herdr-inbox app-feed --ensure [--app-home ABSOLUTE_HOME]` starts one detached writer unless one runs.

| Exit | Meaning |
|---|---|
| 0 | A writer runs. This says nothing about the first feed having been written. |
| 1 | It could not start one. The reason is on stderr, stdout is empty. |
| 2 | Bad arguments, usage on stderr, no I/O. |

Repeated calls are idempotent. The mods call it from `init`, and again at most once per 5 s while the
feed is stale.

| Path under `<appHome>/data/herdr-inbox/` | Meaning |
|---|---|
| `feed.json` | The feed. It stays after the writer exits, and readers see it age. |
| `.feed.tmp` | Staging. Only the lock holder touches it, so a leftover is replaced. Fsynced before the rename. |
| `writer.lock` | Empty. The writer holds `flock` on it for its whole life, so there is one writer per app home. |

The writer:

- opens the inbox database read-only, and never migrates it;
- writes nothing but these files;
- calls neither herdr nor `hc`, and starts no delivery worker;
- polls the database for a change at most 1 s apart, and publishes at least every 5 s, so that
  `generated_at` moves while nothing else does;
- publishes `ok: false` with an `errors` line when the database is missing, busy or not migrated yet.

**Idling out.** It uses `HERDR_APP_HOME=<appHome> herdr-app status`, as Panorama's writer does: every
2 s, bounded to 500 ms. Exit 0 means the app runs. Exit 2 is a confirmed absent window. Anything else
says nothing and restarts the count. After 30 s of confirmed absence the writer exits.

Installing, uninstalling and any generation scheme for these paths belong to B5.

### The worker

The feed writer and the delivery worker are separate processes. The feed reads, the worker delivers.
The mod's `init` also runs `herdr-inbox worker --ensure`, and each verb that leaves an answer to deliver
makes sure of a worker for that answer ([app-commands v1](app-commands-v1.md#delivery)).

There is one worker per herdr server, and it exits about 10 s after that server is gone
(`SERVER_GONE_EXIT`). So on a machine where only herdr-app runs, `worker --ensure` starts a worker that
migrates the database if it needs to, delivers nothing and exits. Until B3 this is expected, and
harmless.

## What depends on what

| What | Holds |
|---|---|
| `group`, `failed`, `alert` with `host: herdr-tui` or `unknown` | From B4. Every row the inbox can hold today is one of these. |
| Answering a TUI-hosted question from the overlay | From B4. It reaches the agent only through a worker of that question's herdr server: with no server running, the answer stays recorded, becomes a ⚠ entry after 60 s, and `check_answers` can still return it. |
| `host: herdr-core` rows | From B2. Until then `ask_operator` and `alert_operator` answer `unavailable` in a core terminal, so no such row exists. The fixtures carry them so that the mods can be built first. |
| Delivery to a core terminal | From B3. The worker's retry loop reads only rows with a pane, so before B3 a core answer would never be sent. It would still show as a ⚠ row, because `failed_deliveries` has no pane rule. |
| A worker with no herdr server | From B3, which extends liveness to herdr-core. |

## Not settled

These change no field above. They need the operator's decision before B2 or B4 are built.

1. **What starts delivery after `answer`.** See [app-commands v1](app-commands-v1.md#delivery).
2. **A core asker on alerts.** The plan's `question_host` table covers questions only. `Asker` is shaped
   to carry a core alert; nothing records one yet, and the blocked scan reads herdr's agents only.
3. **Acting on remote rows.** They are read-only here because the plan's verbs do not cover them.
   Actions on them would be additive.
4. **Where the binary comes from.** The mods need a path to `herdr-inbox`: it is not on `PATH`. A private
   copy under the app home (Panorama's way) sits in no install, so it does not read the plugin's
   `engine` file. A rollback to Python would then not reach the workers it starts.
