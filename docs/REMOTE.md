# Remote machines (design, 0.7.0)

**Status:** design. Operator decisions are open as Q82–Q85 (asked 2026-10-15); see the
end of this file.

## Problem

herdr 0.9.3 can drive saved SSH machines (`herdr machine list --json`,
`herdr --machine <label> <api command>`). An agent on a remote machine runs its own inbox
MCP server **there**. That server writes the remote's `inbox.db` and talks to the remote
herdr server. So its questions never reach the operator's popup on this machine, and
nobody answers them.

What we know about the first remote, `moneylust` (read-only look, 2026-10-15):

- **Connection:** a saved profile (`target` = ssh host `moneylust`, session `default`),
  reachable over ssh with keys.
- **Installed:** Ubuntu with Python 3.12, `uv`, `herdr` and `claude` on `PATH`.
- **Not yet there:** no herdr plugins, no inbox state.

## Principles

1. **One writer per database.** Each machine's `inbox.db` is written only by the inbox
   code on that machine: its MCP servers, popup, CLI and worker. This machine never opens
   a remote database over the network and never writes one.
2. **No cross-machine typing.** Answers are typed into an agent only by the worker of
   the machine the agent runs on, through that machine's own herdr. All the safety rules
   still apply there unchanged: claims, transcript verification, the busy/idle policy and
   the operator-in-pane rule.
3. **Pull, over plain ssh, with the remote's own CLI.** This machine asks each remote
   `inbox export` for its state and sends operator actions as `inbox answer` /
   `inbox dismiss` CLI calls. Nothing on the remote needs to reach back to this machine,
   so no listening ports and no reverse tunnels.
4. **Offline or slow machines never block anything local.** All remote I/O happens in the
   local delivery worker, with hard timeouts. The popup only ever reads the local
   database.

## Where remote questions live locally

The local worker keeps a **mirror** of each enabled machine in the local `inbox.db`, in new
side tables. The `questions` table keeps its 0.1.x column set (a standing rule), so
nothing there changes.

```
remote_machines(machine_id PK, label, target, last_ok_at, last_error, last_try_at)
remote_questions(machine_id, remote_id, group_id, status, question, header, options_json,
                 multi_select, tab_label, pane_id, asked_by, title, created_at, answered_at,
                 answer, delivery_state, delivery_error, updated_at,
                 PRIMARY KEY (machine_id, remote_id))
remote_outbox(machine_id, remote_id, action, detail_json, created_at, sent_at, error)
```

- **Ids:** remote ids are namespaced by machine and shown as `moneylust:Q12`; a local
  `Q12` stays `Q12`. The popup keys remote entries as `("remote", machine_id, group)`,
  never as plain integers, so a cursor can't land on a local row with the same number.
- **Popup list:** remote groups appear in the same list as local ones. Their second line
  reads `moneylust · <tab> · <asked_by>`. The same safety rules apply to them: arming,
  targets not indexes, arrivals appended, closed rows inert.
- **Offline state:** a machine whose last successful pull is older than 3 ticks shows
  one dim line, `moneylust: offline since 10:42 (questions may be stale)`. Its rows stay,
  greyed. Answering them is still allowed and queues in the outbox (below).

## The remote side: `inbox export`

This is a new read-only CLI command:
`inbox export --since <cursor> --json`.

- It prints open and recently changed questions, groups and delivery state as JSON on
  stdout, plus a new cursor (the max `updated` time).
- It never writes, so it is safe on any number of remotes at once.
- Schema stamp check: it opens the DB with `maintenance=False` and does no migration
  unless the remote's own code already did one.

## The pull: the local worker, every tick

For each enabled machine from `herdr machine list --json`:

```
ssh -o BatchMode=yes -o ConnectTimeout=5 -o ServerAliveInterval=5 \
    -o ControlMaster=auto -o ControlPath=<state>/ssh-%C -o ControlPersist=60 \
    <target> '<remote inbox> export --since <cursor> --json'
```

- **`<remote inbox>`:** `uv run --quiet --frozen --no-dev --project <remote plugin root>
  inbox`. The remote plugin root comes from the remote's `herdr plugin list --json`
  (asked over the same ssh) and is cached.
- **Timeouts:** each ssh call is bounded at 10 s. Machines are pulled one after another
  in a thread pool of their own, so one slow machine never delays local deliveries.
- **Applying a pull:** the result is upserted into `remote_questions` in one transaction.
- **New open questions** raise one local notification, `moneylust · Q12 · <tab>`, if
  the operator wants that (Q85).
- **Auth:** ssh keys only (`BatchMode=yes`: never a password or passphrase prompt, and
  never `herdr machine reconnect`).
- **A failing machine** is recorded in `remote_machines.last_error` and retried with
  backoff (5 s, 10 s, 20 s… up to 60 s).

## Answers flowing back

1. The operator answers `moneylust:Q12` in the popup.
2. The popup writes the answer into the local `remote_outbox` (one local DB write) and
   marks the mirror row "answered — sending to moneylust".
3. The worker sends each outbox row as
   `ssh <target> '<remote inbox> answer 12 --detail-json <json> --if-open'`.
   - On the remote this is the normal answer path: its own `service.answer`, its own
     claim and its own worker typing into its own herdr pane, with verification.
   - `--if-open` makes it idempotent: a retry after a lost ssh reply cannot answer twice.
     A question that was already answered or closed there returns its current state and
     leaves the answer as it is.
   - Exit status and JSON say what happened. The outbox row is marked sent, or keeps its
     error and is retried with the same backoff.
4. Delivery state, such as ⏳ pending, ⚠ not delivered or agent gone, comes back on later
   pulls, and the popup shows it as for local rows.
5. **Dismiss, hide and Send now** become outbox actions too (`dismiss`, `hide`,
   `deliver --force`).
6. **Offline:** answering an offline machine's question is allowed. The answer waits in
   the outbox until the machine is back, and the row says so.

## Tab badges, `$inbox_q` and radar on remote tabs

The remote's own inbox code sets `❓N` tab prefixes and the `$inbox_q` pane token on the
**remote** herdr server, exactly as it does locally. When the local herdr TUI shows a
remote machine, those come from the remote server, so this machine does nothing for them.
herdr-radar runs per server: a remote with radar installed uses its own
`inbox radar-sidebar` there.

## Deep links

`inbox open --question moneylust:Q12` resolves against the mirror. `--pane` and
`--session` take a `machine:` prefix the same way.

## Installing and upgrading on a remote

Both plugins must be on the remote: the Claude plugin, so remote agents have
`ask_operator`, and the herdr plugin, so the remote has a worker and its `[[startup]]`.

- **Manual (always possible):** on the remote,
  ```
  claude plugin marketplace add LeaveMyYard/herdr-inbox
  claude plugin install herdr-inbox@herdr-inbox
  herdr plugin install LeaveMyYard/herdr-inbox
  ```
  then restart the remote's Claude sessions.
- **Automated (if the operator wants it, Q83):** `inbox remote install <machine>` runs
  the same commands over ssh (BatchMode) and prints each step. `inbox remote upgrade
  <machine>` runs the `update` variants. Neither restarts remote agents; that stays the
  operator's call.
- **Version skew:** `inbox export` reports its version. If a remote's version is older
  than the export format this machine understands, the popup shows `moneylust: inbox
  0.6.3 — upgrade to see its questions`, and the pull is skipped rather than
  misread.

## Failure modes

| case | what happens |
|---|---|
| machine offline / ssh key missing | rows greyed with "offline since"; answers queue in the outbox; nothing blocks |
| ssh call hangs | killed at 10 s; machine backed off |
| answer sent but reply lost | outbox retries; `--if-open` makes the second send a no-op |
| remote DB busy | the remote CLI's own busy retries; at worst the outbox retries |
| same id on two machines | namespaced: `moneylust:Q12` vs `Q12` |
| remote agent gone | remote marks it agent-gone; the mirror shows it; `d` hides it there too |
| remote plugin missing | the pull fails with "inbox not installed on moneylust" (shown once, dim) |

## Testing

- **Fakes:** a fake `ssh` on `PATH` that runs the remote CLI against a second temp
  database with a second fake herdr. This covers the pull, the outbox, idempotent
  answers, offline backoff and the popup's remote rows.
- **Isolated herdr:** two isolated herdr servers (two sockets) stand in for this machine
  and a remote, with the "ssh" command running locally against the second.
- **moneylust:** read-only until the operator OKs installing there (Q82).

## Operator decisions (asked as Q82–Q85, "inbox author")

- **Q82:** may I install on moneylust for end-to-end testing?
- **Q83:** remote install automated (`inbox remote install`) or manual?
- **Q84:** pull interval (5 s / 15 s / on popup open + 30 s)?
- **Q85:** a local notification for new remote questions?
