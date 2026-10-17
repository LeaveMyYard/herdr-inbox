#!/usr/bin/env python3
"""Bump the `tag =` of the herdr-agent-kit git dependency in Cargo.toml.

Used by .github/workflows/kit-bump.yml. Prints one word on stdout and exits 0:

    absent    Cargo.toml has no herdr-agent-kit git dependency (nothing to do)
    current   the dependency already names TAG (nothing to do)
    bumped    Cargo.toml was rewritten to TAG (the caller refreshes Cargo.lock)

Exits 1 when the dependency exists but its tag cannot be rewritten (it must be
one inline table on one line, with a `tag = "..."` key), or when TAG is not a
plain release tag vX.Y.Z.

Usage: python3 tools/kit_bump.py TAG [CARGO_TOML]

Standard library only (Python 3.11+ for tomllib).
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

KIT = "herdr-agent-kit"
TAG_RE = re.compile(r"^v\d+\.\d+\.\d+$")
LINE_RE = re.compile(r'^(\s*"?herdr-agent-kit"?\s*=\s*\{.*\btag\s*=\s*")([^"]*)(".*)$')


def kit_tags(doc: dict) -> list[str]:
    """Every tag the kit dependency names, across the dependency tables."""
    tables = []
    for owner in (doc, doc.get("workspace", {})):
        for key in ("dependencies", "dev-dependencies", "build-dependencies"):
            tables.append(owner.get(key, {}))
    for target in doc.get("target", {}).values():
        for key in ("dependencies", "dev-dependencies", "build-dependencies"):
            tables.append(target.get(key, {}))
    found = []
    for table in tables:
        dep = table.get(KIT)
        if isinstance(dep, dict) and "git" in dep:
            found.append(dep.get("tag", ""))
    return found


def bump(tag: str, cargo_toml: Path) -> str:
    if not TAG_RE.match(tag):
        raise SystemExit(f"error: {tag!r} is not a release tag vX.Y.Z")
    text = cargo_toml.read_text()
    tags = kit_tags(tomllib.loads(text))
    if not tags:
        return "absent"
    if all(t == tag for t in tags):
        return "current"
    lines = text.splitlines(keepends=True)
    hits = 0
    for i, line in enumerate(lines):
        m = LINE_RE.match(line)
        if m:
            lines[i] = m.group(1) + tag + m.group(3) + ("\n" if line.endswith("\n") else "")
            hits += 1
    if hits != len(tags):
        raise SystemExit(
            f"error: found {len(tags)} {KIT} git dependencies but could rewrite {hits}; "
            'write each as one inline table on one line with tag = "..."'
        )
    new = "".join(lines)
    if any(t != tag for t in kit_tags(tomllib.loads(new))):
        raise SystemExit("error: rewrite did not take; Cargo.toml left unchanged")
    cargo_toml.write_text(new)
    return "bumped"


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    path = Path(argv[2]) if len(argv) == 3 else Path("Cargo.toml")
    print(bump(argv[1], path))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
