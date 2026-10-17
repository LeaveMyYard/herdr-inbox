# Test fixtures

Every fixture here is captured, never written by hand.

- `templates.json`: transcript record templates, copied verbatim from herdr-panorama's
  `tests/fixtures/templates.json` (see its README there); `tests/bridge_scenario.py` builds
  transcripts only from them.
- `composer/*.ansi`: Claude Code input boxes as `herdr pane read` returns them (empty, drafts,
  chips, wrapped text), captured from real panes; the fake herdr in `tests/conftest.py` serves them.
- `radar/*.block.toml`: herdr-radar's own output, produced by `radar/generate.js`
  (see `tests/test_radar.py`).

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
  `tests/test_tui.py`). One file per screen size:

  ```
  {"source": "...", "size": [width, height], "screens": {
    "<screen name>": [
      [[text, fg, bg, bold], ...],   one row per line: the runs of that row
      ...
    ], ...}}
  ```

  A run is text with one style: `fg` and `bg` are `#rrggbb` (lowercase) or `null` (the
  terminal's default colour), `bold` a boolean. `reverse` is resolved (fg and bg swapped, as the
  terminal shows it: the selected tab, chips, the input cursor); the popup draws no other
  attribute. It is what Textual's compositor sends to the
  terminal (`export_screenshot`'s own render), the same shape as herdr-panorama's
  `design_*.json` and `screens/*.json`. Screens are in the order the scenario reaches them.

  Deterministic: fixed sizes and data, `TZ=UTC`, every `HERDR_*`, `INBOX_*` and `TEXTUAL*`
  variable and `NO_COLOR`/`FORCE_COLOR`/`COLORTERM`/`TERM` dropped, a temporary inbox database per scenario, the tests' fake herdr, the arming delays at
  0 (as in the tests), text cursors held on, and a held clock: every row's `created_at` is fixed
  relative to `HELD`, in id order, and the popup's ages are formatted by the real `cli._age` as of `HELD`.
  Nothing of the operator's (inbox, config, transcripts, herdr) is read.

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
