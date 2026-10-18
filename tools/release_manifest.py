#!/usr/bin/env python3
"""Step 3 of the release flow (herdr-panorama docs/plans/rust-rewrite.md §4 item 2): after
vX.Y.Z is published, point main at it.

- writes ``release.toml``: the tag and the sha256 of each target's tarball, read from the
  release's ``SHA256SUMS`` (scripts/install-bin.sh and scripts/launch check downloads against it);
- bumps the plugin version to X.Y.Z in ``herdr-plugin.toml``, ``.claude-plugin/plugin.json``
  (its ``version`` and the per-version MCP venv name) and ``.claude-plugin/marketplace.json``
  (the herdr-inbox entry only).

Until this lands, every install fetches the previous release. ``pyproject.toml`` and
``Cargo.toml`` are bumped by the release PR itself (step 1), not here.

Usage: python3 tools/release_manifest.py TAG SHA256SUMS [ROOT]

Standard library only. Exits 1 with a message when the sums do not hold exactly the
release's targets or a file does not have the one line it expects.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

NAME = "herdr-inbox"
TARGETS = (
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)
TAG_RE = re.compile(r"v(\d+\.\d+\.\d+)")
SUM_RE = re.compile(r"([0-9a-f]{64})  (\S+)")


class ReleaseError(Exception):
    pass


def tarball(version: str, target: str) -> str:
    """The asset name of one target (the release workflow's package step)."""
    return f"{NAME}-v{version}-{target}.tar.gz"


def parse_sums(text: str, version: str) -> dict[str, str]:
    """target -> sha256, from ``sha256sum`` output; exactly the release's targets."""
    by_file = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        m = SUM_RE.fullmatch(line.strip())
        if not m:
            raise ReleaseError(f"not a sha256sum line: {line!r}")
        by_file[m.group(2)] = m.group(1)
    want = {tarball(version, t): t for t in TARGETS}
    if set(by_file) != set(want):
        raise ReleaseError(f"SHA256SUMS names {sorted(by_file)}, expected {sorted(want)}")
    return {want[f]: h for f, h in by_file.items()}


def release_toml(tag: str, sums: dict[str, str]) -> str:
    lines = [
        "# The release installs and updates fetch (plan §4). Written by .github/workflows/release.yml",
        "# (tools/release_manifest.py) after the release was published: never edit by hand.",
        f'tag = "{tag}"',
        "",
        "[sha256]",
    ]
    lines += [f'{t} = "{sums[t]}"' for t in TARGETS]
    return "\n".join(lines) + "\n"


def replace_once(text: str, pattern: str, repl: str, what: str) -> str:
    new, n = re.subn(pattern, repl, text, flags=re.MULTILINE)
    if n != 1:
        raise ReleaseError(f"{what}: expected one match of {pattern!r}, found {n}")
    return new


def bump(root: Path, version: str) -> None:
    p = root / "herdr-plugin.toml"
    p.write_text(replace_once(p.read_text(), r'^version = "[^"]*"$', f'version = "{version}"', p.name))
    p = root / ".claude-plugin" / "plugin.json"
    text = replace_once(p.read_text(), r'^  "version": "[^"]*",$', f'  "version": "{version}",', p.name)
    text = replace_once(text, r'/venv-[0-9][^"]*"', f'/venv-{version}"', p.name + " venv")
    json.loads(text)
    p.write_text(text)
    p = root / ".claude-plugin" / "marketplace.json"
    doc = json.loads(p.read_text())
    entries = [e for e in doc["plugins"] if e.get("name") == NAME]
    if len(entries) != 1:
        raise ReleaseError(f"{p.name}: expected one {NAME} entry")
    text = p.read_text()
    old = json.dumps(entries[0]["version"])
    entry = re.compile(r'("name": "herdr-inbox",\n(?:[^{}]*?\n)??\s*"version": )' + re.escape(old))
    text, n = entry.subn(lambda m: m.group(1) + json.dumps(version), text)
    if n != 1:
        raise ReleaseError(f"{p.name}: could not rewrite the {NAME} version")
    json.loads(text)
    p.write_text(text)


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 3):
        print(__doc__.split("Usage: ")[1].splitlines()[0], file=sys.stderr)
        return 2
    tag, sums_path = argv[0], Path(argv[1])
    root = Path(argv[2]) if len(argv) == 3 else Path(__file__).resolve().parent.parent
    m = TAG_RE.fullmatch(tag)
    try:
        if not m:
            raise ReleaseError(f"not a release tag vX.Y.Z: {tag!r}")
        sums = parse_sums(sums_path.read_text(), m.group(1))
        (root / "release.toml").write_text(release_toml(tag, sums))
        bump(root, m.group(1))
    except (ReleaseError, OSError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
