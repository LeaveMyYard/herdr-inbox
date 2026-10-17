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

    uv run python tools/capture_goldens.py              # rewrite tests/fixtures/screens/
    uv run python tools/capture_goldens.py --out DIR    # write DIR/screens/ instead

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
  terminal's default colour), `bold` a boolean. It is what Textual's compositor sends to the
  terminal (`export_screenshot`'s own render), the same shape as herdr-panorama's
  `design_*.json` and `screens/*.json`. Screens are in the order the scenario reaches them.

  Deterministic: fixed sizes and data, `TZ=UTC`, every `HERDR_*` and `INBOX_*` variable
  dropped, a temporary inbox database per scenario, the tests' fake herdr, the arming delays at
  0 (as in the tests), text cursors held on, and a held clock: every row's `created_at` is fixed
  relative to `HELD`, and the popup's ages are formatted by the real `cli._age` as of `HELD`.
  Nothing of the operator's (inbox, config, transcripts, herdr) is read.
