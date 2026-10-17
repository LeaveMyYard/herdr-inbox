#!/usr/bin/env python3
"""Bump the `tag =` of the herdr-agent-kit git dependency in Cargo.toml.

Used by .github/workflows/kit-bump.yml. Prints one word on stdout and exits 0:

    absent    Cargo.toml has no herdr-agent-kit git dependency (nothing to do)
    pinned    the dependency is pinned by branch or rev, or by a tag that is not
              semver, so it is managed by hand (nothing to do)
    current   the dependency already names TAG (nothing to do)
    newer     the dependency names a newer tag than TAG, a pre-release of a
              newer version included (nothing to do; see --allow-downgrade)
    bumped    Cargo.toml was rewritten to TAG (the caller refreshes Cargo.lock)

Automated runs never move the kit backwards. --allow-downgrade, passed only by
an explicit manual run, rewrites to TAG even when it is older.

Exits 1 when the dependency exists but its tag cannot be rewritten (it must be
one inline table on one line, with a `tag = "..."` key), or when TAG is not a
plain release tag vX.Y.Z.

Usage: python3 tools/kit_bump.py [--allow-downgrade] TAG [CARGO_TOML]

Standard library only (Python 3.11+ for tomllib).
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

KIT = "herdr-agent-kit"
TAG_RE = re.compile(r"v\d+\.\d+\.\d+")
SEMVER_RE = re.compile(r"v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?")
LINE_RE = re.compile(r'^(\s*"?herdr-agent-kit"?\s*=\s*\{.*\btag\s*=\s*")([^"]*)(".*)$')


def semver_key(tag: str) -> tuple | None:
    """A key that orders tags as semver does, or None when TAG is not semver."""
    m = SEMVER_RE.fullmatch(tag)
    if not m:
        return None
    core = tuple(int(x) for x in m.group(1, 2, 3))
    if m.group(4) is None:
        return (*core, 1, ())  # a release sorts after its own pre-releases
    ids = tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in m.group(4).split("."))
    return (*core, 0, ids)


def kit_deps(doc: dict) -> list[dict]:
    """Every herdr-agent-kit git dependency, across the dependency tables."""
    tables = []
    for owner in (doc, doc.get("workspace", {})):
        for key in ("dependencies", "dev-dependencies", "build-dependencies"):
            tables.append(owner.get(key, {}))
    for target in doc.get("target", {}).values():
        for key in ("dependencies", "dev-dependencies", "build-dependencies"):
            tables.append(target.get(key, {}))
    return [t[KIT] for t in tables if isinstance(t.get(KIT), dict) and "git" in t[KIT]]


def bump(tag: str, cargo_toml: Path, allow_downgrade: bool = False) -> str:
    if not TAG_RE.fullmatch(tag):
        raise SystemExit("error: the requested tag is not a release tag vX.Y.Z")
    text = cargo_toml.read_text()
    tags = [dep.get("tag") for dep in kit_deps(tomllib.loads(text))]
    if not tags:
        return "absent"
    if any(not isinstance(t, str) or semver_key(t) is None for t in tags):
        return "pinned"
    if all(t == tag for t in tags):
        return "current"
    if not allow_downgrade and any(semver_key(t) > semver_key(tag) for t in tags):
        return "newer"
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
    if any(dep.get("tag") != tag for dep in kit_deps(tomllib.loads(new))):
        raise SystemExit("error: rewrite did not take; Cargo.toml left unchanged")
    cargo_toml.write_text(new)
    return "bumped"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--allow-downgrade", action="store_true", help="manual runs only")
    ap.add_argument("tag")
    ap.add_argument("cargo_toml", nargs="?", type=Path, default=Path("Cargo.toml"))
    args = ap.parse_args(argv)
    print(bump(args.tag, args.cargo_toml, args.allow_downgrade))
    return 0


if __name__ == "__main__":
    sys.exit(main())
