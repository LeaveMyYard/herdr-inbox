# herdr-app feed v1

The contract between the Rust inbox and the herdr-app mods (`herdr-inbox`, `herdr-inbox-toast`). Rust
owns every decision and every write. A mod reads this one file and acts only through the verbs in
[app-commands v1](app-commands-v1.md). The fixtures are in `tests/fixtures/app-feed/` and
`tests/fixtures/app-commands/`.

**Status: the [installer](#installing) is built (B5); the rest is not yet.** This is the document the writer
(B4) and the mods (B5, B6) are built against. [What depends on what](#what-depends-on-what) says what holds today and what waits for
a later slice.

## Delivery

The writer publishes UTF-8 JSON at `<appHome>/data/herdr-inbox/feed.json`. `<appHome>` is
`$HERDR_APP_HOME`, otherwise `$HOME/.config/herdr-app`, as `appHome()` returns it.

It is a full snapshot, never a delta. Readers see complete files only: the writer fills a sibling
temporary file and renames it over the feed. The file is mode 0600 and its directory 0700. It holds
question text, which the TUI popup also shows, and mods are trusted code.

A mod reloads on `watchFolder(<appHome>/data/herdr-inbox)`, with a slow timer as the fallback, and
reads with `readTextFile`.

**The feed is at most 1 MiB,** because `readTextFile` reads no more (1048576 bytes). Two rules keep it
there, and [a reader rule](#an-incomplete-feed) keeps what they leave out from going unseen.

*Long text is cut.* Questions have no size limit when filed, so the writer cuts these fields, counted in
UTF-8 bytes and at a character boundary, and sets `cut` on the question or row:

| Field | At most |
|---|---|
| `question`, `context`, `answer` | 16 KiB |
| `preview` | 4 KiB |
| `description` | 1 KiB |

Labels, drafts, `asked_by`, `cwd` and `tab_label` are never cut, the first two because an answer depends
on them exactly. With at most 4 questions of 9 options, the cut fields come to about 308 KiB per entry
before JSON escaping, which can grow them, so one large question rarely crowds out the others. That is a
guide, not a bound: anything larger is left out and counted, as below.

A cut question can still be answered: on a machine with only herdr-app there may be no other way. The mod
must show that the text was cut, and offers the whole question through
[`app show`](app-commands-v1.md#showing-one-question-in-full).

*Rows that still do not fit are left out, by need.* The writer fits entries in this order, which is not
the display order:

1. local groups, oldest first;
2. local alerts;
3. `failed` rows;
4. remote rows.

An entry that does not fit in what is left is skipped, and the walk goes on with the next one, so one
huge entry never takes the rows behind it with it. Each skipped entry is counted in `omitted` and the
feed gets an `errors` line. The entries that fit are then published in [display order](#order).

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

The writer reads `entries` and `new_since` in one read transaction, so a mark never runs ahead of its row.

A feed is stale when `generated_at` is missing, in the future, or more than 15 s old. That is a view
rule, not proof the writer died. A stale feed stays on screen. Its tokens still protect every action,
because a verb checks them against the database and not against the file.

### An incomplete feed

This rule is required of every mod that shows the inbox. Whenever `errors` is non-empty or any count in
`omitted` is above 0, the mod:

- shows it, with the counts, for as long as it lasts;
- never shows its empty state while `omitted` has a count above 0;
- offers the rest: each id in `omitted.question_ids` opens with
  [`app show`](app-commands-v1.md#showing-one-question-in-full), which gives the row in full and its token,
  so the overlay can answer it like any other. Where a herdr server runs, the TUI popup has it too.

`herdr-inbox list` is no way out: it prints the first line of each question, at most 100 characters, with no
context and no options.

`entries: []` means an empty inbox only when `ok` is true and every count in `omitted` is 0.

## Feed

| Field | Type | Meaning |
|---|---|---|
| `v` | integer | Exactly 1. |
| `generated_at` | number | When this snapshot was read. |
| `ok` | bool | The local questions and alerts were read. False: `entries` is empty and says nothing about the inbox. |
| `errors` | [string] | What went wrong, as plain text. May be non-empty while `ok` is true: the entries then still hold every local row, and a remote machine's rows or the ones counted in `omitted` are what is missing. |
| `omitted` | Omitted | What was left out to stay under 1 MiB. All 0 normally. |
| `new_since` | NewSince | Where arrivals stand. |
| `entries` | [Entry] | Every row the inbox shows, in the [order below](#order). |

### Omitted

| Field | Type | Meaning |
|---|---|---|
| `groups` | integer | `group` entries left out. |
| `questions` | integer | The open questions in them. |
| `failed` | integer | `failed` entries left out. |
| `alerts` | integer | `alert` entries left out. |
| `remote` | integer | Remote entries left out, of any kind. |
| `question_ids` | [integer] | The left-out open questions' ids, oldest first, at most 1000. |

### NewSince

The highest ids filed so far, open or not.

| Field | Type | Meaning |
|---|---|---|
| `question_id` | integer | The highest question id. 0 if there is none. |
| `alert_id` | integer | The highest alert id. 0 if there is none. |

The toast mod keeps the marks in memory, with the ids it has seen in `omitted.question_ids`:

- Its first good feed sets them, and nothing is announced. That is what keeps a restart or a hot reload
  from announcing old rows.
- From then on, it announces each `group` entry whose `group` is above `question_id`, each `alert` entry
  whose `id` is above `alert_id`, and each id in `omitted.question_ids` it has not seen. Then it takes the
  new marks and ids.
- A row announced as left out gets no second toast if it fits later: its id is below the mark.

A file the reader missed loses nothing: the comparison is against ids, not against the previous file. A
group's id is its first question's id, and every `ask_operator` call that files anything starts a new one,
so an arrival always has a higher id. Remote machines' rows have no ids that compare, and v1 does not
announce them.

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
| `group` | integer | The group id: its first question's, whether or not that one is still open. |
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
| `cut` | bool | Some text of this question was [cut](#delivery). |

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
| `choices` | [string] | Picked labels, in option order, no label twice. |
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
An answer a process holds a live claim on is being sent, so it is no row in the feed.

Only a NULL `delivery_error` waits those 60 s. An answer that is `pending`, which is the usual busy case,
is a row at once, seconds after it was given. So a `failed` row is not in itself a failure: `pending` with
`auto_retry` true is delivery at work, and the mod says that, not ⚠ "failed".

| Field | Type | Meaning |
|---|---|---|
| `question_id` | integer | The question. |
| `group` | integer | Its group. |
| `header` | ?string | Its chip. |
| `question` | string | Its text. |
| `asked_by` | ?string | The agent's role. |
| `answer` | ?string | The answer as it will be sent. |
| `answered_at` | time | When the operator answered. |
| `retries` | integer | How many `retry` verbs were done on it. The token covers it, so a replayed `retry` is refused. |
| `cut` | bool | `question` or `answer` was [cut](#delivery). |
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
| `auto_retry` | bool | The row is in a worker's retry listing (`Store::undelivered`, for a core row `Store::undelivered_core`), so it will try again by itself. False when the agent is gone, when `no live agent` / `conversation changed` / `not verified` has used up its tries, when a core row is stranded (`not delivered: …`, 5 minutes refused or unreadable), and for a row with no pane and no core terminal of this machine. |

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
| `busy` | starts `busy:` | The agent is busy; it sends when the grace ends, 20 s (`BUSY_GRACE`) after `pending_since`. |
| `blocked` | starts `blocked:` | The agent is at a prompt or dialog. |
| `operator_here` | `waiting: the operator is in that pane` | It waits for the operator to leave that pane. |
| `other` | anything else | Show `reason`. |

The mod writes its own wording from these words and does not parse `reason`. It calls `retry` "Send now"
only on a `herdr-tui` row with `wait` `busy` or `busy_idle`: a core row is [never forced](app-commands-v1.md#retry-and-force).

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

`alert_kind: blocked` is always `herdr-tui` in v1, because the blocked scan reads herdr's agents only. An
agent in a core terminal can raise an `agent` alert from B2, through the `alert_host` side table.

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
| `questions` | [Question] | Its open questions. Every `draft` is null. Cut like local ones. |

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
| `cut` | bool | `question` or `answer` was cut. |

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
token = first 32 hex digits of sha256( canonical( the token fields of the entry ) )
```

| `kind` | Token fields |
|---|---|
| `group` | `key`, `group`, `title`, `created_at`, `asker`, `questions` |
| `failed` | `key`, `question_id`, `group`, `header`, `question`, `asked_by`, `answer`, `answered_at`, `retries`, `asker` |
| `alert` | `key`, `id`, `alert_kind`, `severity`, `title`, `message`, `asked_by`, `created_at`, `asker` |

Inside those, only these count: `asker` by `host`, `pane`, `terminal`, `tab_label`, `cwd`; each question by
`id`, `header`, `question`, `options`, `multi_select`, `context`, `asked_by`, `draft`; each option by
`label`, `description`, `preview`; a draft by `choices`, `notes`, `text`. Text is hashed as published, cut
or not, so the cut limits are part of this frozen input. The cut hides no change: nothing writes `question`,
`context`, `options` or a stored answer after filing.

**This list is frozen for v1.** A field added later stays out of it. The writer is long-lived and a verb
starts from whatever binary is current, so after an update the two can differ; a token over "the whole
entry" would then make every action `stale` until the app closed. Anything that must change a token needs
a new version.

`canonical` is that object as JSON with keys sorted by code point at every level, no whitespace, UTF-8, and
non-ASCII characters written as themselves (Python's
`json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`).

A verb builds the entry from the database and computes this again, in the same transaction as its
write. So a token changes when an open question leaves a group, when any text, option or asker changes,
when anyone's draft changes, and when a `retry` is done. It does not change when `delivery` moves, which
happens all the time behind a ⚠ row; the [verb checks that state itself](app-commands-v1.md#the-state-a-verb-checks).

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

Every command below runs [through the launcher](app-commands-v1.md#running-a-command), so
`herdr-inbox app-feed` here means `sh <root>/scripts/launch cli app-feed`.

The writer takes its lessons from Panorama's `docs/protocol/app-feed-runtime-v1.md`: a proven start, a
bounded restart, a cooperative stop, and no signal to any PID.

### The writer

`herdr-inbox app-feed --help` starts nothing, touches no file, exits 0 and prints
`herdr-inbox app-feed v1` as its first line.

`herdr-inbox app-feed --ensure [--app-home ABSOLUTE_HOME]` makes sure one writer of this binary runs.
Repeated calls are idempotent. The mods call it from `init`, and again at most once per 5 s while the feed
is stale. B5 may add optional arguments, such as Panorama's `--install-id`, within v1.

| Exit | Meaning |
|---|---|
| 0 | A writer is proven to run. This says nothing about the first feed having been written. |
| 1 | None could be proven, or starting is deferred or disabled. The reason is on stderr, stdout is empty. |
| 2 | Bad arguments, usage on stderr, no I/O. |

| Path under `<appHome>/data/herdr-inbox/` | Meaning |
|---|---|
| `feed.json` | The feed. It stays after the writer exits, and readers see it age. |
| `.feed.tmp` | Staging. Only the lock holder touches it, so a leftover is replaced. Fsynced before the rename. |
| `ensure.lock` | Empty. `--ensure` holds `flock` on it from its check until the start is proven, so two mods starting together wait for one another, and the second finds the writer running. No failure is counted. It waits at most 7 s; past that it exits 1, spawns nothing and counts nothing. |
| `writer.lock` | Empty. The writer holds `flock` on it for its whole life, so there is one writer per app home. |
| `writer.json` | The writer's record: `{"v":1,"pid":…,"start":…,"binary_sha256":…}`. Diagnostics and busy proof, never a signal target. |
| `control.json` | `{"v":1,"disabled":false}`. Disabled: no writer runs or starts. Absent: `--ensure` and a child refuse, as there is no install to serve. |
| `stop.request` | Empty. A running writer that sees it exits. |
| `start.json` | The restart ledger: `{"v":1,"failures":…,"retry_at_ms":…}`. Absent means no failures. |
| `install.json` | The install record, mode 0600, see [Installing](#installing). The writer does not touch it. |
| `stage/` | The installer's staging. The writer does not touch it. |

The writer:

- opens the inbox database read-only, and never migrates it;
- writes nothing but these files;
- calls neither herdr nor `hc`, and starts no delivery worker;
- polls the database for a change at most 1 s apart, and publishes at least every 5 s, so that
  `generated_at` moves while nothing else does;
- publishes `ok: false` with an `errors` line when the database is missing, busy or not migrated yet.

**Starting.** The child takes `writer.lock`, writes `writer.json`, then acknowledges on a pipe with
`READY v1`. The parent waits at most 5 s. Readiness comes before any database read.

| Outcome | Exit |
|---|---|
| `READY v1` | 0, and the ledger is removed. |
| No acknowledgement, but `writer.json` names this live child and the lock is held | 0, and the ledger is removed. |
| The child died, or nothing proves it | 1, and the attempt stays counted. |
| The lock is held and `writer.json` has this binary's hash | 0, nothing started. |

**Backoff.** Each spawn is counted in `start.json` before it happens. Failure n defers the next spawn by
min(5 s × 2^(n−1), 160 s), and `--ensure` exits 1 without spawning until then. The 8th failure in a row
sets `disabled`, which only a reinstall clears. So a writer that crashes at start cannot be restarted
every 5 s by every mod.

**Stopping.** Nothing is ever killed. A writer checks `control.json` and `stop.request` at each poll,
at most 1 s apart. An uninstall takes `ensure.lock`, sets `disabled` and waits at most 2 s for `writer.lock`
to come free. If it does not, it refuses and leaves everything else in place, with `disabled` set. It holds
`ensure.lock` throughout, so an `--ensure` that has read `disabled: false` finishes first or not at all, and
`--ensure` refuses while `disabled`, so a late call cannot restart a writer behind an uninstall.

**A changed binary.** `--ensure` hashes its own binary. A writer whose `writer.json` has another hash gets a
`stop.request`; `--ensure` waits at most 2 s for the lock, removes the request and starts its own. A writer
also exits by itself when its executable file changes on disk, checked at each publish, and the mod's next
`--ensure` replaces it. The token list above keeps actions working in between.

**Idling out.** It uses `HERDR_APP_HOME=<appHome> herdr-app status`, as Panorama's writer does: every
2 s, bounded to 500 ms. Exit 0 means the app runs. Exit 2 is a confirmed absent window. Anything else
says nothing and restarts the count. After 30 s of confirmed absence the writer exits. `control.json` and
the ledger stay, so the next app start ensures again.

### Installing

`scripts/app-install install --app-home ABSOLUTE_HOME [--plugin-root ROOT]` installs the mods, and upgrades them
when run again. `scripts/app-install uninstall --app-home ABSOLUTE_HOME` takes them out. It needs no package:
`install.json` alone says what it may remove, and it must name only `mods/herdr-inbox*` paths.

An install creates `control.json`, `ensure.lock` and `writer.lock`, and this folder at 0700. Running it again
also clears `disabled` and removes `start.json`. It keeps the feed.

An uninstall removes every path in the table. A file there it does not know stays, and is reported.

The record is written before anything else that matters and removed last, so after a crash at any point
either command can be run again. With no record, a folder holding only these paths is what a crash left:
`install` takes it over and `uninstall` clears it.

An uninstall asks `herdr-app status` what the window shows. It refuses when the shown layout names a mod, when
that layout's file can't be read, and when the answer is anything but exit 0 with a layout or exit 2 for no
window. It asks `--checker` or the `herdr-app` on `PATH`. With neither, it tries to connect to
`<appHome>/herdr-app.sock`: an absent or refused socket means no window, and anything else refuses. With no
window nothing is showing the mods, so it goes ahead and names any layout that still uses them. A mod open in a tab or as an overlay can't be seen this way: the window reports it
missing until that closes.

`install.json` records what the mods need to find the plugin:

| Field | Type | Meaning |
|---|---|---|
| `plugin_root` | string | The root the install ran from. |
| `plugin_moves` | bool | False only for the operator's own git checkout. A herdr install is a git checkout too, but each update makes a new generation under `plugins/github-installations/` and deletes the old one. |
| `claude_config` | ?string | `CLAUDE_CONFIG_DIR` at install, when it was an absolute path. |

The other fields are the installer's own. [app-commands v1](app-commands-v1.md#running-a-command) says how the
mods use these.

### The worker

The feed writer and the delivery worker are separate processes. The feed reads, the worker delivers.
The mod's `init` also runs `herdr-inbox worker --ensure`, and each verb that leaves an answer to deliver
makes sure of the worker of that answer's server ([app-commands v1](app-commands-v1.md#delivery)).

There is one worker per herdr server, and it exits about 10 s after that server is gone
(`SERVER_GONE_EXIT`). So on a machine where only herdr-app runs, that worker migrates the database if it
needs to, delivers nothing and exits. `worker --ensure` also starts the core worker when a core is up:
one per machine, alive while herdr-core is.

## What depends on what

| What | Holds |
|---|---|
| `group`, `failed`, `alert` with `host: herdr-tui` or `unknown` | From B4. Every row the inbox can hold today is one of these. |
| `retries` and `retry` | From B2 and B4. The counter is a `question_ext` column in B2's revision bump, so B4 needs B2. |
| Answering a TUI-hosted question from the overlay | From B4. If herdr proves no server runs on the question's socket, the answer is recorded and the result says `held`, not `queued`: no sender runs, so no attempt is used up. It becomes a ⚠ `unattempted` row after 60 s, `check_answers` can still return it, and that server's worker delivers it once the server is back. |
| `host: herdr-core` rows | From B2, with `question_host`, `alert_host` and `retry_count` in one `MIGRATION_REVISION` bump. A core row has `pane_id` NULL and `herdr_server` `herdr-core`. Until B2, `ask_operator` and `alert_operator` answer `unavailable` in a core terminal, which has no `HERDR_PANE_ID`, so no such row exists. The fixtures carry them so that the mods can be built first. |
| Delivery to a core terminal | From B3. The fixture cases marked `needs: B3` hold from then on. |
| A worker with no herdr server | From B3: the core worker. |
