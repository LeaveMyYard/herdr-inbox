# Remote machines (design, 0.7.0)

**Status:** built in 0.7.0 (#21): `remote_api.py` is the remote side, `remote.py` this
machine's side, and the popup and the worker are wired in. The design was revised after the
design review on #21 (H1–H3, M1–M4, Lows), and the operator answered Q82–Q85 on 2026-10-15;
see the end of this file.

## Problem

herdr 0.9.3 can drive saved SSH machines (`herdr machine list --json`,
`herdr --machine <label> <api command>`). An agent on a remote machine runs its own inbox
MCP server **there**. That server writes the remote's `inbox.db` and talks to the remote
herdr server. So its questions never reach the operator's popup on this machine, and
nobody answers them.

What we know about the first remote, `moneylust` (read-only look, 2026-10-15):

- **Connection:** a saved profile (`target` = ssh host `moneylust`, session `default`),
  reachable over ssh with keys.
- **Installed:** Ubuntu with Python 3.12, plus `uv`, `herdr` and `claude` in
  `~/.local/bin`.
- **Not yet there:** no herdr plugins, no inbox state.

## Principles

1. **One writer per database.** Each machine's `inbox.db` is written only by the inbox
   code on that machine. This machine never opens a remote database.
2. **No cross-machine typing.** Answers are typed into an agent only by the worker of the
   machine the agent runs on, through its own herdr. All the existing rules apply there:
   claims, transcript verification, busy/idle and the operator-in-pane rule.
3. **Pull over plain ssh with the remote's own CLI.** There are no listeners and no
   reverse tunnels. This machine runs two **fixed** commands on the remote, `inbox
   export` and `inbox apply`, and every piece of data travels as JSON on **stdin and
   stdout**, never in a command line (H3).
4. **Offline or slow machines never block anything local.** All remote I/O runs in the
   local delivery worker, with hard timeouts. The popup reads only the local database.
5. **Remote output is untrusted input** (M4). It is size-capped and schema-validated,
   shown through the same escaping as local text, and never executed or interpolated
   into a local command.

## Identity (H1)

- Every inbox database gets a random **`db_id`** (uuid4) the first time it is opened. It
  is stored in `meta`, and recreating the database (reinstall, fresh `XDG_STATE_HOME`,
  `rm`) gives a new one.
- `export` returns it. The local mirror stores it per machine.
- **If a machine's `db_id` changes:**
  - that machine's mirror is wiped and fully re-pulled;
  - every queued action for it **fails**, with a notification: "moneylust's inbox was
    reset — answer again".
- Every question has an identity beyond its number: `qkey` = sha256 of `db_id`,
  `created_at`, `norm` and `session_id`. Every action carries `db_id`, the id and the
  `qkey`. The remote refuses an action whose `qkey` doesn't match the question it names
  (`rejected: not that question`).
- Popup and deep-link keys are `("remote", machine_id, db_id, id)`, and the operator sees
  them as `moneylust:Q12`. A deep-link `machine:` prefix must name an enabled machine
  from `herdr machine list`, or it is refused.

## The local mirror

New side tables, so the `questions` table keeps its 0.1.x column set (a standing rule):

```
remote_machines(machine_id PK, label, target, db_id, cursor_seq, version, uv, herdr,
                plugin_root, last_ok_at, last_error, last_try_at, backoff_until)
remote_questions(machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at,
                 notified_at, PRIMARY KEY (machine_id, remote_id))
                 -- data_json: the validated, capped exported question
remote_outbox(request_id PK, machine_id, db_id, remote_id, qkey, action, detail_json,
              created_at, expires_at, state, result, sent_at, attempts)
```

## Change feed (M1)

- **On the remote:** a side table `changes(seq INTEGER PRIMARY KEY AUTOINCREMENT, qid)`
  is filled by triggers on insert/update/delete of `questions` and `question_ext`. It is
  pruned to the last 10 000 entries.
- **The cursor is `(db_id, seq)`.** It's opaque to this machine, and no clocks are
  compared.
- **Full snapshot:** taken every 60th pull, and whenever `export` reports the cursor
  older than its pruned range. It carries the set of all live ids, so mirror rows whose
  remote question is gone get removed.

## The remote side: `inbox export` and `inbox apply`

Both commands read their request as JSON on stdin and print one JSON object on stdout.

### `export`

- **Request:** `{"cursor": {db_id, seq} | null, "full": bool}`.
- **Reply:** `{version, db_id, seq, full, ids?, questions: [...]}`, covering
  questions that changed since the cursor (or all, for a full snapshot), plus their
  delivery state.
- **No data writes.** The remote's own code may still migrate its schema on open (Low).

### `apply`

- **Request:** `{"request_id", "db_id", "id", "qkey", "action": "answer"|"dismiss"|"hide"|"deliver", "detail"?}`.
- **Idempotence (H2):** the remote records `request_id` with its outcome in a side table
  `applied_requests`. A repeated request returns the recorded outcome and does nothing
  else.
- **Reply:** one of
  - `{"outcome": "applied"}`;
  - `{"outcome": "already_applied"}` (this same request, retried);
  - `{"outcome": "rejected", "reason": "answered with X" | "withdrawn" | "dismissed" | "reset" | "not that question" | "expired"}`.
- **Answering** goes through the remote's normal `service.answer`, so its own worker then
  delivers with every safety rule.

## The ssh call (H3, M3, Lows)

```
ssh -o BatchMode=yes -o ConnectTimeout=5 -o ServerAliveInterval=5 -o ServerAliveCountMax=2 \
    -o ControlMaster=auto -o ControlPath=<state>/ssh-%C -o ControlPersist=60 \
    -- <target> '<uv> run --quiet --frozen --no-dev --project <plugin_root> inbox export|apply'
```

- **The target** comes from herdr's saved profile. It is refused if it starts with `-`
  (as `-oProxyCommand=…` it would run a command **here**) or contains anything other than
  host characters, and it always follows `--`.
- **`<uv>` and `<plugin_root>`** are absolute paths. `<uv>` is found once with a fixed probe
  (`PATH=$HOME/.local/bin:…; command -v uv herdr claude`), since a non-interactive ssh
  gets a minimal PATH. `<plugin_root>` comes from the remote's `herdr plugin list --json`.
  Both are validated against `^/[A-Za-z0-9._/+-]+$` and cached in `remote_machines`.
- **Data never appears in the command line:** everything travels on stdin.
- **Each call is capped at 10 s.** Killing the local ssh client does **not** stop the
  remote command (Low). This is exactly the "lost reply" case, and the request id covers
  it.
- **ControlMaster** uses its own `ControlPath` under the state dir. `%C` is a hash, so it
  stays under macOS's 104-byte socket limit. `ServerAliveCountMax` detects a dead master
  within the timeout.
- **The remote worker** that `apply` starts (via `ensure()`) runs `setsid`-detached and
  gets `HERDR_BIN_PATH=<herdr>`. It must survive the ssh session closing. Checked on
  moneylust during the e2e: if systemd `KillUserProcesses` kills it, the remote's own
  `[[startup]]` and MCP servers restart it, and the design says so.

## Pull and outbox: the local worker

- **Pull:** every `remote_sync_seconds` (default 5, Q84), for each enabled machine, in its
  own thread so a slow machine never delays local deliveries.
  - The worker runs `export` and validates the reply (M4):
    - at most 2 MB in total and 1 000 questions;
    - known keys and types only, with per-field caps (question 4 000 chars, labels 200,
      tab label 120) and control characters stripped;
    - non-conforming rows are dropped and logged.
  - It then applies the reply to the mirror in one transaction.
  - New open questions raise one local notification, `moneylust · Q12 · <tab>` (Q85),
    passed through `herdr.clean`.
- **Outbox:** FIFO **per question** (H2). A queued answer and a later dismiss for the
  same question go in order, and a failed head blocks the rest of that question, but no
  other question.
  - **Before sending**, the row is re-validated against the latest pull (M2): `db_id` and
    `qkey` must match, and the question must be open in the mirror. Otherwise the row is
    rejected locally.
  - **Expiry (M2):** a queued action expires after `remote_answer_expiry_hours` (default
    **12 h**). It then fails with a notification instead of sending a stale answer into a
    conversation that has moved on.
  - **Results:** `applied` / `already_applied` mark the row done. `rejected` is shown on
    the mirror row ("moneylust rejected your answer: answered with X") and raises a
    notification.
  - **Errors:** ssh and transport errors retry with backoff (5, 10, 20 … up to 60 s).
  - **Version skew (Low):** an argparse exit 2, or an export-reported version too old for
    `apply`, is **permanent**: "upgrade moneylust (`inbox remote upgrade moneylust`)".
    Sends are gated on the export's version.

## The popup

- Remote groups appear in the same list. Their second line reads
  `moneylust · <tab> · <asked_by>`.
- All existing safety rules apply: arming, targets not indexes, arrivals appended, closed
  rows inert.
- **Offline machine (M2):** one dim line, `moneylust: offline since 10:42 (3 h)`. Its rows
  are greyed and show their age. Answering them queues the action, and the row says
  "queued for moneylust · expires 22:42".
- **Text:** every remote string goes through `escape()` (Rich) or `markup=False`, as local
  ones do.

## Tab badges, `$inbox_q` and radar on remote tabs

The remote's own inbox sets `❓N` and `$inbox_q` on the remote herdr server, and they show
in the local herdr TUI's view of that machine. herdr-radar is per server.

## Installing and upgrading on a remote (Q83: automated)

`inbox remote install <machine> [--ref <git ref>]` and `inbox remote upgrade <machine>`:

1. probe `uv`, `herdr` and `claude` with the fixed command; absolute paths only;
2. `<claude> plugin marketplace add LeaveMyYard/herdr-inbox[#ref]` (update on upgrade);
3. `<claude> plugin install herdr-inbox@herdr-inbox` (update on upgrade);
4. `<herdr> plugin install -y [--ref <ref>] LeaveMyYard/herdr-inbox`;
5. `<herdr> plugin list --json` to confirm the version.

- Each step is printed, and the run stops at the first failure.
- The ref must match `[A-Za-z0-9._/-]+`.
- Neither command restarts remote agents; that stays the operator's call.
- The manual equivalent is the same commands run on the remote.

## Failure modes

| case | what happens |
|---|---|
| machine offline / no ssh key | rows greyed with age; actions queue until they expire (12 h) |
| ssh call hangs | killed at 10 s; the remote may still finish, and `request_id` makes the retry return `already_applied` |
| answer sent but reply lost | retry with the same `request_id` → `already_applied` |
| someone else answered first | `rejected: answered with X`, shown and notified |
| remote inbox recreated | `db_id` changes: mirror wiped, queued actions fail with "answer again" |
| same number on two machines / after a reset | keyed by `(machine, db_id, id)` plus `qkey` |
| remote too old | permanent "upgrade moneylust"; no retry storm |
| hostile or garbled export | capped, validated, escaped; bad rows dropped; nothing executed |
| remote worker killed at logout | restarted by the remote's `[[startup]]` / MCP servers |

## Testing

- **Fakes:** a fake `ssh` runs the remote CLI against a second temp database and a second
  fake herdr. It covers export, apply, ids/`db_id`/`qkey`, request ids and lost replies,
  FIFO, expiry, offline backoff, version skew, hostile exports and popup rows.
- **Isolated herdr:** two isolated servers on two sockets, with "ssh" running locally
  against the second.
- **moneylust:** the e2e install is OKed (Q82). Test questions are removed afterwards.

## Operator decisions (Q82–Q85, answered 2026-10-15)

- **Q82 — yes, install on moneylust now** for end-to-end testing: both plugins, nothing
  else on its herdr server beyond the plugin, no costly agents beyond what the e2e needs,
  and test questions cleaned up afterwards.
- **Q83 — automated:** `inbox remote install|upgrade <machine>` (above).
- **Q84 — configurable, default 5 s:** `remote_sync_seconds = 5` in the inbox
  `config.toml`.
- **Q85 — yes:** a new remote question raises a herdr notification here, once per question.
- **Answer expiry (the review's M2):** default **12 h**, set by
  `remote_answer_expiry_hours`. I picked the value; the operator can change it.
