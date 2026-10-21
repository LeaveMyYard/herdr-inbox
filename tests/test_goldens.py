"""The inbox's behaviour goldens (herdr-panorama docs/plans/rust-rewrite.md §5.2 item 2, the
popup's app screens, and item 5, the delivery decision table) still describe the code:
tools/capture_goldens.py run again, into a temporary directory, writes the same files as the
committed tests/fixtures/screens/ and tests/fixtures/decisions.json. The decision table is checked
by running only the tests the golden names (``--golden-tests``; every other test drives no
traced decision). A failure is a behaviour change. If it is a deliberate one, regenerate (the
whole suite, recorded) and say why in the PR:

    uv run python tools/capture_goldens.py
"""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
FIX = ROOT / "tests" / "fixtures"


def first_difference(want: dict, got: dict) -> str:
    new = [n for n in got["screens"] if n not in want["screens"]]
    if new:
        return f"new screens {new!r}"
    for name in want["screens"]:
        a, b = want["screens"][name], got["screens"].get(name)
        if b is None:
            return f"screen {name!r} is gone"
        if len(a) != len(b):
            return f"screen {name!r} has {len(b)} rows, not {len(a)}"
        for y, (ra, rb) in enumerate(zip(a, b)):
            if ra != rb:
                return (f"screen {name!r} row {y}:\n  want {''.join(r[0] for r in ra)!r}\n"
                        f"  got  {''.join(r[0] for r in rb)!r}\n  want runs {ra}\n  got runs  {rb}")
    return "screen list or header differs"


def decision_difference(want: dict, got: dict) -> str:
    def key(r):
        return json.dumps({k: v for k, v in r.items() if k != "tests"}, sort_keys=True, ensure_ascii=False)
    a, b = {key(r): r for r in want["rows"]}, {key(r): r for r in got["rows"]}
    lines = [f"  gone: {k}  (from {a[k]['tests'][:3]})" for k in sorted(a.keys() - b.keys())][:5]
    lines += [f"  new:  {k}  (from {b[k]['tests'][:3]})" for k in sorted(b.keys() - a.keys())][:5]
    lines += [f"  same row, other tests: {k}" for k in sorted(a.keys() & b.keys()) if a[k] != b[k]][:3]
    return "decision rows differ:\n" + "\n".join(lines) if lines else "header differs"


@pytest.mark.parametrize("kind", ["screens", "decisions", "schema", "landed", "store", "herdr"])
def test_the_goldens_are_what_the_code_does(tmp_path, kind):
    env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
    extra = ["--golden-tests"] if kind == "decisions" else []
    res = subprocess.run([sys.executable, str(ROOT / "tools" / "capture_goldens.py"), "--out", str(tmp_path),
                          "--only", kind, *extra], env=env, cwd=ROOT, capture_output=True, text=True, timeout=900)
    assert res.returncode == 0, (res.stdout + res.stderr)[-4000:]
    if kind == "decisions":
        a, b = (FIX / "decisions.json").read_text(), (tmp_path / "decisions.json").read_text()
        if a != b:
            pytest.fail(f"decisions.json: {decision_difference(json.loads(a), json.loads(b))}\n"
                        "a behaviour change; if deliberate: uv run python tools/capture_goldens.py")
        return
    if kind == "schema":  # what the Rust hooks check a database against (src/store/readiness.rs)
        assert (FIX / "schema.json").read_text() == (tmp_path / "schema.json").read_text(), (
            "the schema changed: bump store.MIGRATION_REVISION, then uv run python tools/capture_goldens.py "
            "--only schema")
        return
    if kind == "herdr":  # the Rust screen parsers replay it (src/delivery/screen)
        assert (FIX / "herdr-screens.json").read_text() == (tmp_path / "herdr-screens.json").read_text(), (
            "herdr.composer_state / panel_focused changed: if deliberate, uv run python tools/capture_goldens.py "
            "--only herdr")
        return
    if kind == "landed":  # the Rust landing check replays it (src/claude/landed)
        assert (FIX / "landed.json").read_text() == (tmp_path / "landed.json").read_text(), (
            "transcript.landed_kinds changed: if deliberate, uv run python tools/capture_goldens.py --only landed")
        return
    if kind == "store":  # the Rust store replays them (tests/store/*_golden.rs)
        for name in ("store-questions.json", "store-delivery.json", "store-alerts.json", "store-mirror.json", "store-outbox.json"):
            assert (FIX / name).read_text() == (tmp_path / name).read_text(), (
                f"{name}: a Store read or write changed: if deliberate, "
                "uv run python tools/capture_goldens.py --only store")
        return
    want = sorted(p.name for p in (FIX / kind).glob("*.json"))
    got = sorted(p.name for p in (tmp_path / kind).glob("*.json"))
    assert want and got == want
    for name in want:
        a, b = (FIX / kind / name).read_text(), (tmp_path / kind / name).read_text()
        if a != b:
            pytest.fail(f"{kind}/{name}: {first_difference(json.loads(a), json.loads(b))}\n"
                        "a behaviour change; if deliberate: uv run python tools/capture_goldens.py")


def test_the_decision_table_is_a_function():
    """inbox-core::decide must reproduce the table, so the same inputs (path, mode, every lookup,
    the state before) never lead to two outcomes: a conflict means a lookup is not recorded."""
    rows = json.loads((FIX / "decisions.json").read_text())["rows"]
    outcomes: dict[str, set[str]] = {}
    for r in rows:
        key = json.dumps({k: r.get(k) for k in ("path", "inputs", "observed", "question", "before")}, sort_keys=True)
        outcomes.setdefault(key, set()).add(json.dumps({k: r.get(k) for k in ("after", "action")}, sort_keys=True))
    conflicts = {k: v for k, v in outcomes.items() if len(v) > 1}
    assert not conflicts, "\n".join(f"{k}\n  -> {sorted(v)}" for k, v in list(conflicts.items())[:3])
