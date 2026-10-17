#!/usr/bin/env python3
"""Validate the Python-to-Rust test ledger and count its rows by status.

The ledger (tests/ledger/test-ledger.csv, plan docs/plans/rust-rewrite.md §5.2
item 7) maps every collected Python test to its Rust test, or retires it with a
reason. A phase is done when its rows are filled.

Usage:
    python3 tools/ledger_check.py [LEDGER] [--collected FILE]

--collected takes the output of `pytest --collect-only -q` (run with every
HERDR_* variable unset) and also checks that the ledger has exactly one row per
collected test. Exit status is 0 when the ledger is valid, 1 otherwise.

Standard library only, so it runs without the project's venv.
"""

from __future__ import annotations

import argparse
import csv
import sys
from collections import Counter
from pathlib import Path

COLUMNS = ["python_test_id", "python_file", "rust_test", "status", "retired_reason", "phase"]
STATUSES = ("mapped", "retired", "todo")
PHASES = {"", "P0a", "P0b", "P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8"}
DEFAULT_LEDGER = Path(__file__).resolve().parent.parent / "tests" / "ledger" / "test-ledger.csv"


def read_collected(path: Path) -> list[str]:
    """Test ids from `pytest --collect-only -q` output (the summary lines are skipped)."""
    return [line.strip() for line in path.read_text().splitlines() if "::" in line]


def check(ledger: Path, collected: Path | None) -> tuple[list[str], Counter[str]]:
    errors: list[str] = []
    counts: Counter[str] = Counter()
    with ledger.open(newline="") as fh:
        reader = csv.reader(fh)
        header = next(reader, None)
        if header != COLUMNS:
            return [f"header is {header!r}, expected {COLUMNS!r}"], counts
        seen: set[str] = set()
        for lineno, row in enumerate(reader, start=2):
            if len(row) != len(COLUMNS):
                errors.append(f"line {lineno}: {len(row)} fields, expected {len(COLUMNS)}")
                continue
            rec = dict(zip(COLUMNS, row))
            tid, status = rec["python_test_id"], rec["status"]
            if not tid:
                errors.append(f"line {lineno}: empty python_test_id")
            elif tid in seen:
                errors.append(f"line {lineno}: duplicate python_test_id {tid}")
            seen.add(tid)
            if rec["python_file"] != tid.split("::", 1)[0]:
                errors.append(f"line {lineno}: python_file {rec['python_file']!r} does not match {tid}")
            if status not in STATUSES:
                errors.append(f"line {lineno}: status {status!r} not one of {STATUSES}")
            elif status == "mapped" and not rec["rust_test"]:
                errors.append(f"line {lineno}: mapped without rust_test")
            elif status == "retired" and not rec["retired_reason"]:
                errors.append(f"line {lineno}: retired without retired_reason")
            elif status == "todo" and (rec["rust_test"] or rec["retired_reason"]):
                errors.append(f"line {lineno}: todo row has rust_test or retired_reason set")
            if status != "retired" and rec["retired_reason"]:
                errors.append(f"line {lineno}: retired_reason set on a {status!r} row")
            if rec["phase"] not in PHASES:
                errors.append(f"line {lineno}: phase {rec['phase']!r} not one of {sorted(PHASES)}")
            counts[status] += 1
    if collected is not None:
        want = read_collected(collected)
        missing = sorted(set(want) - seen)
        extra = sorted(seen - set(want))
        errors += [f"collected but not in the ledger: {t}" for t in missing]
        errors += [f"in the ledger but not collected: {t}" for t in extra]
    return errors, counts


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("ledger", nargs="?", type=Path, default=DEFAULT_LEDGER)
    ap.add_argument("--collected", type=Path, help="output of `pytest --collect-only -q`")
    args = ap.parse_args(argv)
    errors, counts = check(args.ledger, args.collected)
    for err in errors:
        print(f"error: {err}", file=sys.stderr)
    total = sum(counts.values())
    print(f"total {total}: " + ", ".join(f"{s} {counts[s]}" for s in STATUSES))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
