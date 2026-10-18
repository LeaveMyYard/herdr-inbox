"""tools/release_manifest.py, step 3 of the release flow (.github/workflows/release.yml): the
release.toml it writes and the manifest versions it bumps, on copies of the real manifests."""

import hashlib
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
TOOL = ROOT / "tools" / "release_manifest.py"
TARGETS = ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl", "x86_64-apple-darwin", "aarch64-apple-darwin")
MANIFESTS = ("herdr-plugin.toml", ".claude-plugin/plugin.json", ".claude-plugin/marketplace.json")


@pytest.fixture
def repo(tmp_path):
    for rel in MANIFESTS:
        (tmp_path / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(ROOT / rel, tmp_path / rel)
    return tmp_path


def sums(version="9.8.7", targets=TARGETS, extra=()):
    lines = [f"{hashlib.sha256(t.encode()).hexdigest()}  herdr-inbox-v{version}-{t}.tar.gz" for t in targets]
    return "\n".join([*lines, *extra]) + "\n"


def run(repo, tag, text):
    (repo / "SHA256SUMS").write_text(text)
    return subprocess.run([sys.executable, "-I", str(TOOL), tag, str(repo / "SHA256SUMS"), str(repo)],
                          capture_output=True, text=True)


def snapshot(repo):
    return {rel: (repo / rel).read_text() for rel in MANIFESTS}


def test_release_toml_and_exactly_the_version_lines(repo):
    old = (ROOT / "herdr-plugin.toml").read_text().split('version = "')[1].split('"')[0]
    before = snapshot(repo)
    res = run(repo, "v9.8.7", sums())
    assert res.returncode == 0, res.stderr
    doc = tomllib.loads((repo / "release.toml").read_text())
    assert doc["tag"] == "v9.8.7"
    assert doc["sha256"] == {t: hashlib.sha256(t.encode()).hexdigest() for t in TARGETS}
    changed = []
    for rel, text in before.items():
        new = (repo / rel).read_text().splitlines()
        assert len(new) == len(text.splitlines())
        changed += [(rel, a, b) for a, b in zip(text.splitlines(), new) if a != b]
    assert len(changed) == 4, changed  # herdr-plugin version, plugin.json version + venv, marketplace entry
    assert all(old in a and "9.8.7" in b for _, a, b in changed)
    market = (repo / ".claude-plugin/marketplace.json").read_text()
    assert market.count('"version": "9.8.7"') == 1  # the herdr-inbox entry only, never herdr-inbox-ui


@pytest.mark.parametrize("case", ["missing target", "extra file", "duplicate", "binary mode", "other version"])
def test_bad_sums_are_refused_and_nothing_is_written(repo, case):
    text = {
        "missing target": sums(targets=TARGETS[:3]),
        "extra file": sums(extra=[f"{'0' * 64}  herdr-inbox-v9.8.7-riscv64.tar.gz"]),
        "duplicate": sums(extra=[f"{'1' * 64}  herdr-inbox-v9.8.7-{TARGETS[0]}.tar.gz"]),
        "binary mode": sums().replace("  herdr-inbox", " *herdr-inbox", 1),
        "other version": sums(version="9.8.6"),
    }[case]
    before = snapshot(repo)
    res = run(repo, "v9.8.7", text)
    assert res.returncode == 1 and res.stderr.startswith("error: ")
    assert snapshot(repo) == before and not (repo / "release.toml").exists()


@pytest.mark.parametrize("tag", ["9.8.7", "v9.8", "v9.8.7-rc1", "v9.8.7 "])
def test_a_tag_that_is_not_a_release_is_refused(repo, tag):
    res = run(repo, tag, sums())
    assert res.returncode == 1 and not (repo / "release.toml").exists()


def test_a_manifest_it_cannot_rewrite_leaves_every_file_untouched(repo):
    marketplace = repo / ".claude-plugin/marketplace.json"
    marketplace.write_text(marketplace.read_text().replace('"name": "herdr-inbox",', '"name": "renamed",'))
    before = snapshot(repo)
    res = run(repo, "v9.8.7", sums())
    assert res.returncode == 1
    assert snapshot(repo) == before and not (repo / "release.toml").exists()
