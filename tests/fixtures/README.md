# Test fixtures

Every fixture here is captured, never written by hand, except `popup-deviations.json` (where the
Rust popup deliberately differs from Python, no capture can produce it), `ancestor-replay.json`
(synthetic, see below), `app-feed/` and `app-commands/` (contract examples, see below) and
`migrate-rust.json` (what Rust added to the schema past Python's revision: compiled into the
migration by `src/store/owned.rs`, and applied by `tests/test_core_rows.py` to build the database
an old Python process meets).

- `templates.json`: transcript record templates, copied verbatim from herdr-panorama's
  `tests/fixtures/templates.json` (see its README there); `tests/bridge_scenario.py` builds
  transcripts only from them.
- `composer/*.ansi`: Claude Code input boxes as `herdr pane read` returns them (empty, drafts,
  chips, wrapped text), captured from real panes; the fake herdr in `tests/conftest.py` serves them.
- `radar/*.block.toml`: herdr-radar's own output, produced by `radar/generate.js`
  (see `tests/test_radar.py`).

## herdr-app contract examples

`app-feed/*.json` and `app-commands/commands.json` are examples of `docs/protocol/app-feed-v1.md` and
`docs/protocol/app-commands-v1.md`, with fake data only. No code produces them yet: the mods are built
against them, and the writer and the verbs must reproduce them. Their tokens are real, by the document's
formula. Core rows are there before any code can file one.

- `every-kind.json`: every entry kind, both hosts and an old row with neither, every delivery state and
  `wait` word, two gone-agent answers (the popup's `gone` row), both alert kinds, a core alert, a group
  whose first question has closed.
- `stale-token.json`: the same world after an agent withdrew a question of group 14. That group's token
  differs, and `commands.json`'s `stale-token` case sends the old one.
- `empty.json`: nothing open. `partial.json`: `ok` with `errors`, `omitted` and a cut question.
  `omitted-only.json`: no entries, and not an empty inbox. `not-migrated.json`: `ok: false`.
- `unknown-vocabulary.json`: a newer writer's words and fields, which a v1 reader must survive. The tokens
  ignore the extra fields.
- `commands.json`: `{"source", "feeds", "cases": [{name, about, world, server, host, session, needs, argv,
  exit, stdout, effects, replay_of}]}`, covering every verb, refusal code and `closed` word. `world` is the
  feed the database matches, unless `about` starts `Setup:`, which names what changed after that feed was
  read. `server` is the question's stored `herdr_server`, `session` whether it has a session, and `host` what
  the probe says: `up`, `gone` (proven) or `unproven`. `needs: "B3"` marks a case that holds only once core
  delivery exists. `effects` is what starts after the commit, in order: `ensure_worker` (a server), then
  `sender` (arguments, or null) with `sender_socket`. `replay_of` names a case that ran first with the same
  arguments.

## Behaviour goldens (rust-rewrite plan §5.2)

Captured from the running Python code by `tools/capture_goldens.py` and checked by
`tests/test_goldens.py`, which captures again into a temporary directory and compares the
files byte for byte. A failure there is a behaviour change; if it is deliberate, regenerate and
say why in the PR:

    uv run python tools/capture_goldens.py                    # rewrite screens/ and decisions.json
    uv run python tools/capture_goldens.py --only decisions   # the decision table alone (whole suite)
    uv run python tools/capture_goldens.py --out DIR          # write under DIR instead

- `screens/*.json` (§5.2 item 2, app-screen goldens; the inbox popup's spec, since it has no JS
  design goldens): the popup (`InboxApp`) run headless with Textual's `run_test` through the
  scenarios in `capture_goldens.py` (taken from `tests/make_screenshots.py` and
  `tests/test_tui.py`): `popup.json` (110x30), `popup-narrow.json` (40x20), and
  `popup-extra.json` (110x30, `extra_screens`: the views the others do not reach, such as
  pending and gone-agent answers, a note, an empty note box, a tab closed under the operator,
  a long list that scrolls and a blocked alert), and `popup-remote.json` (110x30,
  `tools/popup_remote_screens.py`, P6-D2: a remote machine's rows from the local mirror — its
  groups, queued, sending, expired and refused actions, an answer it cannot deliver, its
  alerts — its question and alerts opened, the remote dismiss confirmation and the
  placeholders; the mirror's clock is held at `HELD` too, and an action's expiry is past the
  real clock the popup compares it with):

  ```
  {"source": "...", "size": [width, height], "screens": {
    "<screen name>": [
      [[text, fg, bg, bold], ...],   one row per line: the runs of that row
      ...
    ], ...}}
  ```

  A run is text with one style: `fg` and `bg` are `#rrggbb` (lowercase) or `null` (the
  terminal's default colour), `bold` a boolean. `reverse` is resolved (fg and bg swapped, as the
  terminal shows it: the selected tab, chips, the input cursor). Italic (notes, typed text)
  is drawn but not recorded. It is what Textual's compositor sends to the
  terminal (`export_screenshot`'s own render), the same shape as herdr-panorama's
  `design_*.json` and `screens/*.json`. Screens are in the order the scenario reaches them.

  Deterministic: fixed sizes and data, `TZ=UTC`, every `HERDR_*`, `INBOX_*` and `TEXTUAL*`
  variable and `NO_COLOR`/`FORCE_COLOR`/`COLORTERM`/`TERM` dropped, a temporary inbox database per scenario, the tests' fake herdr, the arming delays at
  0 (as in the tests), text cursors held on, and a held clock: every row's `created_at` is fixed
  relative to `HELD`, in id order, and the popup's ages are formatted by the real `cli._age` as of `HELD`.
  Nothing of the operator's (inbox, config, transcripts, herdr) is read.

  `crates/inbox-core/tests/popup_screens.rs` drives the Rust popup model through the same
  scenarios (`tests/screens/`) and compares `inbox_core::popup_view::render` with every screen
  cell for cell. One exception: the empty popup is design 2C there (Q179, `design_empty_inbox.json`
  below), so `popup.json`'s `empty` screen is Python's alone.

- `decisions.json` (§5.2 item 5, the delivery decision table; `inbox-core::decide` must
  reproduce it): every delivery decision the test suite drives, recorded by the test-only trace
  `herdr_inbox.decision_trace` (`RECORDER` is None in every real process; only the pytest plugin
  `tests/decision_recorder.py`, loaded with `-p tests.decision_recorder`, sets it). Regenerating
  runs the whole suite under that plugin; the check (`--golden-tests`) runs only the tests the
  golden names. Traced paths: `deliver_batch` (one frame per pane: worker retries, the popup's
  sends, Send now), `hook` (`hook.deliver`: PostToolBatch/Stop and the rewake watcher's
  delivery), `verify_hook` (`verify_hook_deliveries`), `verify_job` (`inbox verify`) and
  `rewake` (one row per watcher poll that looked at the pane). Decisions made in a subprocess
  the tests start (a real worker, a hook run as a command) are not recorded.
  The table holds only what the tests drive: a rule no test reaches has no row (today, among
  others, a row with no pane, a subagent's hook firing, and a stale claim taken over).
  `test_the_decision_table_is_a_function` checks that the same inputs never lead to two
  outcomes (a conflict means a lookup is not recorded), and the CI job `decision-table`
  regenerates the table from the whole suite, so rows from new tests are caught too.

  ```
  {"source": "...", "files": [test files that drove a row], "rows": [
    {"path": "deliver_batch", "inputs": {"retry": false, "force": false, "verify_later": false},
     "observed": {pane-level lookups, in the order the code made them},
     "question": {this question's own lookups},
     "before": {delivery state}, "after": {delivery state},
     "tests": [node ids that drove this row]},
    {"path": "rewake", "inputs": {}, "observed": {"pending": true, "ready": "go", "hook_sent": true},
     "action": "wake", "tests": [...]},
    ...]}
  ```

  One row per question per decision, deduplicated, sorted by content (one row per line). A key
  missing from `observed`/`question` was not looked up (the code decided before it needed it).
  Lookups: `pane` (the row has a pane), `agent` and `agent_again` (herdr's agent: `kind`,
  `status`, `focused`, `session_known`, or null), `claimed`, `conversation_changed`, `watched`
  (idle and the rewake watcher alive), `hook_live`, `hook_misses`, `busy_attempts`,
  `within_busy_grace`, `panel_focused`, `transcript_known`, `landed` (segment indexes of
  `service.expected_segments` already in the conversation; 2 is the long-answer reference),
  `composer` (`empty`/`draft`/`unknown`), `prompt_error`, `verify_transcript`,
  `landed_after_typing`, `moved_elsewhere` (its conversation runs in another pane: moved there),
  `full_answer_in_message` (false: only the long-answer reference is typed),
  `pane_runs_older_session`, `pane_reports_another_session`, `already_typed_into_older_session`,
  `landed_anywhere_after_typing` and `older_session_watched` (herdr-inbox#53: an older session of the
  asking conversation never takes or verifies its answer); the hook adds `own_session`, `has_candidates`, `schema_ready`,
  `in_this_message`, `recorded`; `verify_hook` adds `verify_time_up`.
  A delivery state is `{"status", "state", ...}` with `state` one of `unattempted`, `claimed`
  (`claim_stale`), `pending` (`reason`), `agent_gone` (`reason`), `failed` (`reason`),
  `delivered`, plus the flags that are set (`verified`, `acknowledged`, `hook_sent`,
  `pending_since`, `agent_gone_marked`, `moved_to_another_pane`) and the counters that are not
  zero (`attempts`, `hook_misses`, `busy_attempts`). Reasons and errors have pane ids, times,
  paths and session ids (or their 8-character prefixes) replaced by `<pane>`, `<time>`, `<path>`,
  `<session>`. Deterministic: two full recorded runs give the
  same file.

- `store-questions.json` (`--only store`): `store.Store`'s question reads and writes run over
  the script `STORE_OPS` in `capture_goldens.py`. Each step records its arguments, its result
  (`dataclasses.asdict` of a question) and the rows of `questions`, `question_ext` and
  `question_groups` after it. A `sql` step sets up a row with raw SQL (a fixed time,
  `STORE_FIXED`, which stays unmasked); every other time, exactly `YYYY-MM-DDTHH:MM:SS+00:00`,
  is masked as `<time>`, and claims as `sending:<time>:<token>`. `options` holds what
  `normalize_options` returns (or raises) for malformed options, and `texts` holds
  `normalize`/`clean_title` of a few texts. `tests/store/questions_golden.rs` runs the same
  script on the Rust store and must match every step.
- `store-delivery.json` (`--only store`): the same for the delivery state, over
  `STORE_DELIVERY_OPS`: claims, pending and busy misses, failures and the piles the worker reads,
  agents gone and panes moved, hook sends verified or not, legacy checks, notifications,
  continuations and liveness. Tables `questions`, `question_ext` and `sessions` after each
  step. `"$claim:N"` stands for the last claim on question N; `<x>_s` arguments are seconds.
  `tests/store/delivery_golden.rs` replays it.
- `store-alerts.json` (`--only store`): `alerts.py` over `STORE_ALERT_OPS`: agent alerts
  (raise, already open, withdraw, acknowledge, report) and the blocked-prompt scan. herdr is
  stubbed at its function boundary (`server_id`, `agent list`, `tab_label`,
  `time.monotonic`). What `alerts.py` does to herdr is recorded per step as `effects`
  (`notify`, `markers`). Tables `alerts` and `blocked_panes` after each step.
  `tests/store/alerts_golden.rs` replays it.
- `store-mirror.json` (`--only store`): `remote.py`'s mirror, the puller's side, over
  `STORE_MIRROR_OPS`. Pulls run with the ssh stubbed (`_paths`, `call`) and the clock injected
  (`_now`), covering failures and backoff, full and incremental snapshots, a reset, and alerts
  being acknowledged. Also the popup's reads and stuck answers. The four mirror tables are
  dumped after each step, unmasked (time is injected), and the export call's cursor and every
  notification are recorded as `effects`. `vectors`: `validate_export` over
  `STORE_EXPORT_VECTORS`, as the rows it keeps or the error it raises.
  `tests/store/mirror_golden.rs` replays both.
- `store-outbox.json` (`--only store`): `remote.py`'s outbox over `STORE_OUTBOX_OPS`.
  Covers enqueueing and its refusals, flushes against every reply a remote can give, retries
  and backoff, re-validation against the mirror, expiry with the status query, a flush stopped
  by Python raising, an unsafe target, the sweep, and the popup's `clearable`/`mark_seen`/
  `outbox_rows`. ssh is scripted at `subprocess.run`, each `inbox apply` answered by the
  step's next `replies` entry; `uuid4` counts up. The outbox and the mirrored alerts are
  dumped after each step, unmasked. Every ssh call (with the row's `sending_at` while an apply
  runs) and every notification are recorded. `tests/store/outbox_golden.rs` replays it.

## The CLI golden (P5-G)

- `cli.json` (`--only cli`, `tools/capture_cli.py`): Python's `cli.main` run in-process over the
  script `CLI_OPS`. Each `cli` step records the command line, its exit code, stdout and stderr
  (argparse's help and usage errors included), its `effects` (what it asked of herdr: `token`,
  `pane_tabs`, `tab_label`, `rename_tab`; of the delivery: `deliver` with the question ids; of the
  worker: `ensure` with the server) and the `questions`, `question_ext` and `tab_decor` rows after
  it (times as `<time>`; `"unchanged"` when they are as the step before left them). herdr, `service.deliver_batch` and the worker are stubbed at their
  function boundary; a stubbed delivery writes the step's `delivery` outcome onto the rows it is
  handed. A step may set `config.toml` (`config`), write transcripts (`files`), record sessions
  (`sessions`) and set the caller's `CLAUDE_CODE_SESSION_ID`/`HERDR_PANE_ID` (`me`). `submit`
  steps run `service.submit` without delivery (the popup's path, with drafts of every shape);
  `preamble` steps run the real `deliver_batch` with every claim refused, so only its preamble
  shows: the worker made sure of, and the answers kept for this herdr server (`result`). An
  `open` step also records the focus request it left (`request`, nonce and time masked) and
  `open_popup` with the env it was given; `popup_open` holds the popup's lock meanwhile.
  `deeplink` steps are the popup's side (`tui.on_mount`): a request file (`request`, written with
  nonce `n1`, `age` seconds old, or `raw` text) taken with the step's `nonce`, then resolved
  (`machines`: herdr's enabled machines), recording the request, whether the file was `left`,
  `resolved` (`[["group", n] | ["remote", key], qid]`) and `describe`.
  `sync` steps are `sync-tokens` command lines with herdr's tabs (`all_tabs`, their `labels`).
  `radar` steps are `radar-sidebar` command lines over herdr's config built around one of
  herdr-radar's own blocks (`herdr_config`: `block` from `radar/`, wrapped as
  `tests/test_radar.py` does, with `replace`, `crlf`, or our cells already in for `edited`) and
  radar's own `config.toml` (`radar_config`); they record the config left (`herdr_config`) and
  whether a backup was written. herdr's side is stubbed as `radar_installed`, `radar_config_dir`
  and `reload_config`. `radar_missing` steps are the popup's read-only hint check
  (`radar.missing`). `export` and `apply` steps (`tools/capture_cli_remote.py`) give their
  request as `stdin`; their stdout is masked (times other than the seeds' fixed `CREATED` as
  `<time>`, the version as `<version>`, a random `db_id` as `<db_id>`, a JSON parser's reason as
  `<reason>`), and `spawn_deliver` records the detached `inbox deliver` (Python
  `remote_api.spawn_delivery`, stubbed). The last part runs on a database of its own (`fresh`):
  after it the tables are `questions`, `question_ext`, `question_groups`, `alerts`,
  `applied_requests`, `changes` and the database's `db_id`. The Rust replay also hands every
  export, Rust's as printed and Python's as kept, to the puller's validation and mirror
  (`tests/cli/remote.rs`): both must pass and mirror the same rows. Ages
  are relative to now; in stderr the work directory is `<work>` and a TOML parser's reason
  `<reason>`. Python 3.13's argparse: older ones wrap the top usage line differently, so
  `test_goldens[cli]` runs on 3.13 only. `tests/cli/golden.rs` replays it on the Rust CLI.

## The popup transition golden (plan herdr-app-inbox §2.3, P6-A)

- `popup-transitions.json`: the popup's key-safety contract as data, `(state, event) ->
  (state, effects)`. Captured from the Python popup (`tui.InboxApp` under Textual's
  `run_test`) by `tools/capture_popup_transitions.py` over the scenarios in
  `tools/scenarios_popup.py`, and checked by `tests/test_goldens.py`
  (`test_the_popup_transition_golden_is_what_the_popup_does`). Regenerate after a deliberate
  behaviour change: `uv run python tools/capture_popup_transitions.py`, then
  `python3 tools/popup_scenario_list.py` (one Rust test per scenario). Two models replay
  it: the Rust popup model (`crates/inbox-core/tests/popup_transitions.rs`) and the herdr-app
  mod's JavaScript model (Part B, B6). Either one must produce, after every step, exactly the
  `state` and the `effects` the step records.

  Capture: the real delays, on a held clock in integer milliseconds that only the scenario
  moves (it starts at 10000). Polls happen only when a scenario says so. The popup's IO is cut
  at the model's boundary: drafts, sends, dismissals, hides, acknowledgements, go-to-agent
  requests, notifications and exits are recorded as effects. A send is recorded at once and
  reported back by a later `sent` event, the way a detached send reports. A go-to-agent is
  reported back by a `focused` event.

  ```
  {"schema": 1, "source": "...",
   "delays_ms": {"arm": 400, "row_arm": 1000, "grace": 500, "release": 500, "go": 500},
   "scenarios": [{"name": "<test file>.<test>[.<variant>]", "doc": "...", "steps": [
     {"at": <ms>, "event": <event>, "effects": [<effect>, ...], "state": <state>}, ...]}]}
  ```

  The first step's event is always `open`. Each following event happens at its `at`.

  Events:
  - `{"open": {"focus_pane": str|null, "snapshot": <snapshot>, "deeplink"?: {"target":
    <entry>|null, "qid": int|null, "hold": bool, "missing": str}}}`: the popup opens. A
    `deeplink` was resolved by the IO layer. When its `target` is null, `missing` is the
    notice to show.
  - `{"key": <name>}`: Textual's key names (`enter`, `escape`, `tab`, `shift+tab`, `up`,
    `down`, `left`, `right`, `home`, `end`, `backspace`, `delete`, `space`) or one printable
    character.
  - `{"poll": <snapshot>}`: the periodic poll read the database.
  - `{"sent": {"done": [qid], "wanted": int, "snapshot": <snapshot>}}`: the oldest `send`
    effect was recorded. `done` holds the questions it answered, and `snapshot` is the
    database after it.
  - `{"data": <snapshot>}`: the database changed and the popup did not poll. Python reads
    the database live whenever the operator acts (which of a group's questions are open when
    it opens, what a dismissal names, whether a draft or a dismissal took). So a model takes
    this read as its copy of the data, while rows and placeholders still change only at polls.
    The IO layer sends one before each key, or whenever the database changed.
  - `{"focused": {"pane": str, "error": str|null}}`: the oldest `go_to` finished.

  `<snapshot>` is the database as the popup reads it, with only the fields the model decides
  with:
  `{"questions": [{"id", "group", "status", "pane_id", "options": [label], "multi_select",
  "draft": <detail>|null, "delivery_error", "delivered": bool, "has_preview": bool}], "open": [qid] (store.list),
  "failed": [qid] (store.failed_deliveries, in order), "alerts": [{"id", "kind", "status",
  "pane_id"}], "open_alerts": [aid] (in order)}`. `<detail>` is `{"choices": [label],
  "notes": {label: note}, "text": str|null}`.

  Effects, in the order the popup made them:
  - `notify {text, severity}` (`information`/`warning`);
  - `draft {qid, detail|null}`;
  - `send {"kind": "answer", qid, detail}`: a lone question, sent at once;
  - `send {"kind": "submit", qids}`: the drafts among them;
  - `send {"kind": "retry", qid}`: a ⚠ entry's Retry or Send now, with force;
  - `dismiss {qid}`, `hide {qid}`, `ack {alert}`;
  - `go_to {pane, graced}`: if not `graced`, the IO layer first checks that the agent is not
    waiting at a prompt;
  - `exit {}`: close now;
  - `quit {}`: close once in-flight sends are recorded.

  After `exit` or `quit` the state is `{"running": false}` and the scenario ends.

  The model's copy of the database between events. A model keeps its own effects' writes in
  that copy, as Python reads them back from the database at its next action. The next `poll`,
  `sent` or `data` snapshot then replaces the copy. The writes:
  - `draft`: the question's `draft`, if it is open;
  - `send answer`: its `draft` is set and its `status` becomes `answered`, if it is open;
  - `send submit`: every listed question that is open and drafted becomes `answered`;
  - `dismiss`: an open question becomes `dismissed`;
  - `hide`: the answer leaves `failed`;
  - `ack`: the alert becomes `acked` and leaves `open_alerts`.

  An `answered` or `dismissed` question also leaves `open`. Rows and placeholders change only
  at polls. The Submit tab sends the drafts it read when it appeared or last reloaded, not the
  drafts as they are at the key (`edge.submit_reads_the_drafts_it_showed`).

  `<state>` holds the following fields. A time of `null` means never.
  - `running`, `mode` (`empty`/`list`/`group`/`failed`/`alert`);
  - `focus`: which widget has Textual's focus, and so sees a key first:
    - `view`: the view on screen;
    - `box`: its open text box;
    - `chips`: the tab row, where only the app's keys work (←/→, Tab, `d`, `g`, Esc); it also
      holds the focus in the empty view;
    - `choices`: the view's scrolling list, where keys reach the view;
    - `preview`: a question's preview pane (`has_preview`), where keys reach the view.

    Tab or Shift+Tab inside a text box moves the focus off the box: Tab to `preview` beside a
    preview pane, else to `chips` under the tab row, else to the view; Shift+Tab to `choices`. The box stays open, and the next
    keys act on the view or the app (`edge.tab_in_a_box_*`). In the list, Tab moves between
    `view` and `choices`;
  - `entries`: list rows in order, placeholders included. An entry is `group:<id>`,
    `failed:<qid>`, `gone` (the gone-agents summary row) or `alert:<id>`;
  - `closed`: `{entry: reason}`, the inert placeholders;
  - `fresh_until`: `{entry: ms}`, when a row that arrived in the background takes Enter and
    digits;
  - `list_cursor`, `direct`, `held`, `entry`;
  - `tabs`: `q:<qid>` (the open group's question tabs, placeholders included);
  - `current`: `q:<qid>` or `submit`;
  - `gone`: `{tab: reason}`;
  - `single`;
  - `grace_until`, `view_at`, `last_key_at`, `prev_key_at`;
  - `confirm`: what the open "Dismiss … without answering?" names, or null;
  - `view`: one of the following.
    - `{"kind": "none"}`.
    - `{"kind": "list", "cursor": entry|null, "armed_at"}`.
    - `{"kind": "question", "qid", "in_group", "last", "cursor": "opt:<i>"|"other"|"chat"|"submit",
      "picked": [i], "notes": {"<i>": note}, "text", "editing": "note:<i>"|"text"|null,
      "input": {"value", "cursor"}|null, "armed_at"}`.
    - `{"kind": "review", "items": [qid], "cursor": "item:<qid>"|"submit"|"cancel"|null,
      "armed_at"}`.
    - `{"kind": "gone", "ref", "reason"}`.
    - `{"kind": "failed", "qid", "cursor": 0|1, "armed_at"}`.
    - `{"kind": "alert", "alert", "label": null|str, "armed_at"}`: `label` is a remote
      machine's name for its alert (`<label>:A<id>`).

  The comparison is by JSON value: object key order does not count.

  Remote machines (P6-D2; scenarios `remote_sync.*`, from `tests/test_remote_sync.py`, and
  `remote_edge.*`, captured by `tools/popup_remote.py` and `tools/scenarios_popup_remote.py`).
  The popup reads a remote machine from the local mirror only; the delivery worker talks to it.
  A snapshot has a `remote` part only when the mirror has rows (so the other scenarios carry
  none):
  `"remote": {"machines": [{"machine_id", "label": str|null}], "groups": [{"key",
  "machine_id", "questions": [<rq>]}] (remote.open_groups: a question with an answer or a
  dismissal already queued is left out), "mirror": [<ref>] (every mirrored question, open or
  not), "alerts": [{"machine_id", "id", "kind", "status", "pane_id", "db_id", "akey"}]
  (remote.open_alerts), "outbox": [{"request_id", "machine_id", "action", "remote_id",
  "state", "clearable": bool}] (remote.outbox_rows, oldest first; `clearable` is
  remote.clearable at the time of the read), "problems": [{"machine_id", "remote_id"}]
  (remote.problem_rows)}`. `<rq>` is a `<snapshot>` question's fields (`id` the remote id,
  `group` the remote group; no draft) and its identity's `db_id` and `qkey`. `<ref>` is
  `{"machine_id", "remote_id", "db_id", "qkey"}`: which question on which machine, as shown.

  Their entries: `remote:<machine>|<db id>|<group>` (an open group, numbered),
  `routbox:<request id>` (a queued, sending, expired, failed or refused action),
  `rproblem:<machine>|<remote id>` (an answer the machine cannot deliver),
  `ralert:<machine>|<alert id>` (an alert: unnumbered, with the local alerts). Alerts come
  first (the local ones, then the remote ones), the remote groups, outbox rows and problems
  last. Mode `remote`: one remote question in a question view, opened from its row (its
  group's first open question). It has no tabs (Tab, Shift+Tab, ←/→ never switch or release a
  held deep link, in Python; Tab moves Textual's focus: view, option list, the open box, the
  preview pane), and `d` and `g` do nothing. Enter on a `routbox` or `rproblem` row only says "This is on
  a remote machine; d clears it here". `g` on a remote row in the list says "That agent runs on
  a remote machine: open it from herdr's machine view", except an `rproblem` row: nothing.

  Their effects:
  - `remote_answer {<ref>…, detail}`: the shown question's answer, queued under its
    identity as shown. If the mirror holds another question there now (`db_id` or `qkey`
    differ) or none, nothing is queued: `notify "not queued: that question changed — reopen
    it"` (or `that remote question is gone`), and the view stays;
  - `remote_dismiss {<ref>…}`: one per question of the row, as they were when `d` was pressed;
  - `remote_ack {machine_id, alert_id, db_id, akey}`;
  - `clear_outbox {request_id, expire}`: `d` on a `clearable` row; `expire` when it was
    still queued (past its expiry), settled as failed first;
  - `clear_problem {machine_id, remote_id}`.

  The model's copy: `remote_answer` and `remote_dismiss` take the question out of its group
  (the group goes with its last question); `remote_ack` takes the alert out of `alerts`;
  `clear_outbox` and `clear_problem` take their row out. A remote answer that leaves its
  group empty closes the row (`answered · sending to <label>`); a confirmed dismissal closes it
  (`dismissing on <label>`); `d` on outbox, problem and alert rows closes them (`cleared`,
  `acknowledged`). Reasons at a poll: `answered or closed`, `sent`, `delivered or cleared`,
  `closed` (a remote alert). A poll in mode `remote` whose question left its group puts an
  inert `{"kind": "gone", "ref": "<label>:Q<id>", "reason": "closed on the remote machine"}` on
  screen with the grace window (the label as shown when it opened); an open remote alert that
  ends shows `<label>:A<id>` (the label read afresh). For a machine without a label, see
  deviation 3 below.

  What a model does not reproduce, because the golden cannot hold it:
  - Shift selections and the clipboard in a text box (`shift+←`, `ctrl+x`/`c`/`v`). The box
    keeps only the editing keys listed in `inbox-core::popup::TextBox`.
  - ↑/↓ on `choices` or `preview` scroll in Textual when the list overflows the popup. The
    golden's popup (100×30) never overflows. The models never scroll, on purpose: the cursor
    moves, as it does when nothing overflows, and the JS model must keep it so.
  - A second box opened while a box is open but does not have the focus. This happens with
    `n`, or with Enter or Space on "Type something." after Tab or Shift+Tab out of a box.
    Python crashes there (Textual `DuplicateIds`); the model replaces the box.

  Deliberate deviations from Python (main's decision, P6-D2). These five remote-row behaviours
  are Python bugs, and the Rust popup fixes them instead of copying them. The captured goldens
  above stay Python's own, byte for byte: `tools/capture_popup_transitions.py` and
  `tools/capture_goldens.py` cannot produce the fixed behaviour, and
  `tests/test_goldens.py` keeps checking them against the Python code.

  What Rust must do instead is written by hand in `popup-deviations.json`. A model replays a
  golden through it:

  ```
  {"schema": 1, "source": "...",
   "transitions": [{"scenario": <name>, "quirk": <n>, "why": "...", "from_step": <i>,
                    "steps": [<step>...]}],
   "screens": [{"file": "popup-remote", "screen": <name>, "quirk": <n>, "why": "...",
                "row": <y>, "runs": [<run>...]}]}
  ```

  A `transitions` entry replaces the scenario's steps from `from_step` to its end (the tail can
  be shorter than Python's: the scenario ends differently). A `screens` entry replaces one row
  of a screen. `crates/inbox-core/tests/popup_transitions.rs` and `popup_screens.rs` apply them,
  and fail when an entry no longer differs from the golden, so a Python fix followed by a new
  capture asks for the entry to go. What no capture reaches is in
  `crates/inbox-core/tests/popup_deviations.rs`.

  1. **"Chat about this" on a remote question** (scenario `remote_edge.chat_about_this`). Python
     goes to the remote question's pane id on this machine, which can be an unrelated local
     agent. Rust focuses nothing, stays in the view and says `That agent runs on <machine>: open
     it from herdr's machine view`. A safety fix: no `go_to`, so no `focused` and no quit.
  2. **The label of an acknowledged remote alert** (screen `remote alert acknowledged in the
     list`). Python draws `✓ None:A1 · acknowledged`, because it looks the label up among the open
     alerts. Rust takes the machine from the row's key: `✓ project1:A1 · acknowledged`.
  3. **A machine without a label** (screen `an unlabelled machine's question`, scenario
     `remote_edge.unlabelled_machine`). Python writes `None:Q1` in the question's title and
     placeholder, and `A1` in an alert's placeholder. Rust goes by the machine id everywhere
     (`m2:Q1`, `m1:A1`), as the other remote text already does; an empty label counts as none.
  4. **A held deep link into a remote question** (scenario
     `remote_sync.deep_link_to_a_remote_question.held`). Python never releases it, although the
     hint says "press Tab or →". Rust releases it as in a group: Tab or → after the release
     delay, and that key does nothing more. While held, Shift+Tab and ← do nothing either (no
     focus move). Once released, Tab moves the focus again.
  5. **`y` on a remote group whose mirror changed after `d`** (a question replaced or gone; no
     capture: it ends the popup). Python's `LookupError` escapes the confirmation and crashes
     it. Rust queues the dismissals up to that question, then says `not queued: …` and leaves the
     row open.

  Known gap, kept as parity (main's decision, not a deviation): a remote alert's acknowledgement
  goes under the alert the mirror holds now, not the one on screen. Its row's key is
  `<machine>|<id>`, with no `db_id`, so after a remote reset that reuses the number, `d` acknowledges
  an alert the operator never saw (Python `_ack_entry` re-reads `alert_by_key`; the model does the
  same). No question or dismissal is affected: those go under the identity shown.

  Known gap, kept as parity: clearing an undeliverable answer (`clear_problem`) has no identity
  check. It goes by machine and id only (Python `clear_problem` too), so after a remote reset that
  reuses the number it stamps the question the mirror holds now. It only hides a row here (the
  remote keeps the answer for check_answers), and the new question's own problem row stays hidden.

  A write refused after the model moved on (P6-D3; no capture: Python writes first and never has
  to take anything back). The model announces a remote action and moves on in the same step, and
  the IO layer's write comes after it. When that write is refused (the mirror changed since the
  read, the outbox row started sending, the database failed), the IO layer stops the batch there,
  ahead of the announcement, and sends `RemoteRefused`. The model returns to the state it had when
  the action started, as if it had refused by itself: an answer's view with what was typed, the
  row open, the popup running. The one exception is an alert's view, which ends on the list, as
  Python's `on_alert_view_ack` does after a `LookupError`; a database error ends there too, where
  Python's `guarded` stays in the view (an alert has no typed text to lose). Dismissals queued
  before the refused one stay queued, and the fresh read leaves the row with the questions that
  are left. If that read fails too, the model's copy still shows them as open, so the IO layer
  refuses a second answer or dismissal of a question that has one queued or done (`not queued:
  that question already has an action queued`). Python's `enqueue` has no such check; its popup
  reads live. `crates/inbox-core/tests/popup_refused.rs` and `src/tui/run/tests/remote.rs` pin it.

## Design goldens (rust-rewrite plan §5.2 item 1)

- `design_empty_inbox.json`: design 2C, the empty inbox (the tumbleweed in the desert, Q179),
  captured by node from the design's own cell-grid renderer `inboxC(f)` in herdr-panorama's
  `docs/design/herdr-tui.js` (the file's sha256 is in `source`). It is not Python's: the popup
  gains this screen in the port. Regenerate with:

      node tools/capture_empty_inbox.mjs ../herdr-panorama/docs/design/herdr-tui.js \
        > tests/fixtures/design_empty_inbox.json

  ```
  {"source": "...", "size": [64, 20], "period": 42, "C": {the design's palette},
   "frames": [{"f": 0, "rows": [[[text, fg, bg, bold], ...], ...]}, ...]}
  ```

  Frames 0 to `period` (the last shows the loop back to frame 0; the script also checks that
  frame `f + period` is frame `f`). A run is text whose cells share `fg`, `bg` (`#rrggbb`) and
  `bold`, exactly as the design's grid holds them: a blank keeps the fg it was written with.
  `crates/inbox-core/tests/design_empty_inbox.rs` compares `inbox_core::empty_inbox::frame`
  with every frame cell for cell.

  The design lives in another repo, so CI cannot capture it again: whenever herdr-panorama's
  `docs/design/herdr-tui.js` changes, re-run the capture and compare its sha256 with the one in
  `source` (a different file means the fixture, and maybe the scene, must follow).

## The worker's evidence and the resumed-ancestor replay (P5-E2)

- `evidence.json` (`python3 -I tools/capture_evidence.py`): how Python's `transcript.own_answer_qids`
  splits shell commands (its `shlex` setup: the tokens, or the error shlex raises) and which question
  ids each answers, and what `own_answer_succeeded` reads from CLI outputs. The commands are the ones
  tests/test_failed_pile.py uses plus the splitter's corners (quotes, escapes, comments, punctuation
  runs, assignments, `uv run` flags). `src/claude/evidence/{shlex,shell}/tests.rs` replay it. An id
  past 64 bits is dropped on the Rust side (it never names a question).
- `ancestor-replay.json`: synthetic, written by hand (no Python run produces it): the shape of
  herdr-inbox #53's incident with placeholder ids, names, texts and times only. A session continued in
  a newer one; herdr resumed the older one in the asking pane; six answers asked by the newer session
  were typed into the older one; a reconcile that counted ancestors as proof marked them verified.
  `tests/worker_duties/ancestor_replay.rs` replays it through whole worker ticks and checks no row is
  ever verified, delivered or seen.
