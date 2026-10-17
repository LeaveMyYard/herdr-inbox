# Contributing to herdr-inbox

Thanks for helping. Bug reports, fixes, docs and ideas are all welcome. This project follows the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Feature freeze while the Rust port is in progress

herdr-inbox is being ported from Python to Rust, as laid out in the plan in [herdr-panorama's docs/plans/rust-rewrite.md](https://github.com/LeaveMyYard/herdr-panorama/blob/main/docs/plans/rust-rewrite.md) (§5.5).
While a component is being ported, its Python version takes **bug fixes only**: a feature built in
Python would have to be built twice. Fixes are welcome; for a new feature, please open an issue
first to talk it through. It may have to wait for the Rust version, or land there directly.

## Build and run the tests

You need [uv](https://docs.astral.sh/uv/) (Python 3.11 or newer) and, for the Rust crates, the
toolchain pinned in `rust-toolchain.toml`.

**Unset every `HERDR_*` variable first.** If one points at a live herdr (a socket, a pane, a state
dir), a test can reach your real session. Run this in the shell you test from:

```sh
unset $(env | sed -n 's/^\(HERDR_[A-Za-z0-9_]*\)=.*/\1/p') 2>/dev/null || true
```

Then:

```sh
uv sync --frozen
uv run pytest                          # Python tests

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked        # Rust tests
```

The tests use a fake herdr and temporary state, never your real herdr. CI runs the same on Linux
and macOS.

## Pull requests

- Keep a PR small and about one thing. Describe what changed and why; link the issue it fixes.
- A fix comes with a test that fails without it.
- Adding or renaming a Python test needs a row in `tests/ledger/test-ledger.csv`; CI checks it
  (`python3 tools/ledger_check.py`).
- Commit messages: a short imperative summary line, then the reason in the body if it is not obvious.
- CI must be green. The maintainer reviews every PR and may ask for changes; please answer in the PR.
- Start as a draft if the work is not finished.

## Questions and bugs

Ask questions and float ideas in [Discussions](https://github.com/LeaveMyYard/herdr-inbox/discussions).
Bugs and concrete requests go in an [issue](https://github.com/LeaveMyYard/herdr-inbox/issues/new/choose).
Looking for a first contribution? Try the
[good first issue](https://github.com/LeaveMyYard/herdr-inbox/labels/good%20first%20issue) and
[help wanted](https://github.com/LeaveMyYard/herdr-inbox/labels/help%20wanted) labels. Report security
problems privately, see [SECURITY.md](SECURITY.md).
