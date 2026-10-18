#!/usr/bin/env python3
"""Structure check (COMMON.md "Code structure"): every Rust source file (`src/`
and `crates/*/src/`; the Python package under `src/herdr_inbox/` is not Rust and
is not checked) has at most
300 lines of non-test code, no inline test module and no test function.

An inline test module is a `mod x { ... }` under an outer cfg that names
`test` (`#[cfg(test)]`, `#[cfg(all(test, unix))]`, also when rustfmt wraps the
attribute over several lines; a `test` under `not(...)` does not count), one
whose body starts with an inner `#![cfg(test)]`, or any inline `mod tests {`
(also `mod r#tests`, and with the `{` on the next line). Comments (`//` and
`/* */`), blank lines and other attributes may sit between the cfg and the
module. A test function is any `#[test]` or `#[<path>::test]` (`tokio::test`)
outside a test module.
Comments and the insides of string and char literals are blanked before
anything is matched, so a brace or a `test` in them counts for nothing.

Test files (`tests.rs`, anything under a `tests/` directory) are not counted.
Exceptions are listed in `.github/structure-allowlist.txt`, one path per line
with a reason after `#`. The list may only shrink: an entry whose file now
complies (or no longer exists) fails the check, and with `--base <git ref>`
(CI passes the PR's base branch) so does an entry the base does not have.

Copied from herdr-agent-kit (v0.1.0, .github/scripts/structure-check.py); the
changes: which directories hold Rust sources (`rust_sources`), and a base
branch without an allowlist counts as an empty one (`base_entries`).

usage: structure-check.py [--base <git ref>] | --self-test
"""

import pathlib
import re
import subprocess
import sys

MAX_LINES = 300
ROOT = pathlib.Path(__file__).resolve().parents[2]
ALLOWLIST_REL = ".github/structure-allowlist.txt"
ALLOWLIST = ROOT / ALLOWLIST_REL
# `mod x {`, `mod r#x {`, or `mod x` with its `{` on the next line (`#[rustfmt::skip]`)
INLINE = re.compile(r"^(pub(\([^)]*\))?\s+)?mod\s+(r#)?(\w+)\s*(\{|$)")
# An attribute whose cfg predicate names `test`: cfg(test), cfg(all(test, unix)), ...
TEST_CFG = re.compile(r"^#!?\[\s*cfg\s*\(.*\btest\b")
NOT = re.compile(r"\bnot\s*\(")
# A test function's attribute: #[test], #[tokio::test], #[tokio::test(flavor = ...)]
TEST_FN = re.compile(r"^#\[\s*(\w+\s*::\s*)*test\s*[(\]]")


def is_test_cfg(attr: str) -> bool:
    """Does the attribute's cfg predicate name `test` other than under a
    `not(...)` (`cfg(not(test))` is code that is *not* compiled for tests)?"""
    while m := NOT.search(attr):
        depth, end = 0, len(attr)
        for at in range(m.end() - 1, len(attr)):
            depth += {"(": 1, ")": -1}.get(attr[at], 0)
            if depth == 0:
                end = at + 1
                break
        attr = attr[: m.start()] + attr[end:]
    return bool(TEST_CFG.match(attr))
RAW_STRING = re.compile(r'(?<![\w])[bc]?r(#*)"')


def literal_end(src: str, i: int) -> int | None:
    """Where the string or char literal starting at `src[i]` ends (the index
    after it), or `None` if no literal starts there (a lifetime `'a`)."""
    raw = RAW_STRING.match(src, i) if src[i] in "bcr" else None
    if raw:
        end = src.find('"' + raw.group(1), raw.end())
        return len(src) if end < 0 else end + 1 + len(raw.group(1))
    if src[i] == '"':
        j = i + 1
        while j < len(src) and src[j] != '"':
            j += 2 if src[j] == "\\" else 1
        return j + 1
    if src[i] == "'" and (src.startswith("\\", i + 1) or src.startswith("'", i + 2)):
        end = src.find("'", i + 3 if src[i + 1] == "\\" else i + 2)
        return len(src) if end < 0 else end + 1
    return None


def blank_comments_and_literals(lines: list[str]) -> list[str]:
    """The lines with comments removed and literals emptied (`"a}"` -> `""`),
    line breaks kept. Block comments nest, as in Rust."""
    src = "\n".join(lines)
    out = []
    i = 0
    while i < len(src):
        if src.startswith("//", i):
            end = src.find("\n", i)
            i = len(src) if end < 0 else end
            continue
        if src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < len(src) and depth:
                step = src[j : j + 2]
                depth += {"/*": 1, "*/": -1}.get(step, 0)
                j += 2 if step in ("/*", "*/") else 1
            out.append("\n" * src.count("\n", i, j))
            i = j
            continue
        end = literal_end(src, i) if src[i] in "\"'bcr" else None
        if end is None:
            out.append(src[i])
            i += 1
            continue
        out.append('""' + "\n" * src.count("\n", i, end))
        i = end
    return "".join(out).split("\n")


def split_attribute(text: str) -> tuple[str, str] | None:
    """`text` (starting with `#`) split after its first attribute's closing
    `]`; `None` while that attribute is still open."""
    depth = 0
    for at, c in enumerate(text):
        depth += {"[": 1, "]": -1}.get(c, 0)
        if c == "]" and depth == 0:
            return text[: at + 1], text[at + 1 :].strip()
    return None


def problems_in(lines: list[str]) -> list[str]:
    """The violations in a file's lines: inline test modules, test functions,
    and more than MAX_LINES lines outside test modules."""
    out = []
    code = 0
    depth = 0  # brace depth inside an inline test module (0: not in one)
    pending_test_cfg = False
    attr = ""  # an attribute still open on an earlier line
    head = None  # (line, is a test module) of a `mod x` whose `{` is still to come
    opened = None  # (line, depth) of an inline module whose body may still start with #![cfg(test)]

    def test_module(start: int, n: int, braces: int) -> None:
        nonlocal code, depth, opened
        out.append(f"inline test module at line {start}")
        code -= n - start + 1
        depth, opened = braces, None

    def enter_module(start: int, n: int, body: str, braces: int, is_test: bool) -> None:
        nonlocal opened
        if is_test or (body.startswith("#!") and is_test_cfg(body)):
            test_module(start, n, braces)
        elif not body:
            opened = (start, braces)

    for n, line in enumerate(blank_comments_and_literals(lines), 1):
        if depth:
            depth += line.count("{") - line.count("}")
            continue
        code += 1
        s = f"{attr} {line.strip()}".strip() if attr else line.strip()
        attr = ""
        braces = line.count("{") - line.count("}")
        if head and s:
            start, is_test = head
            head = None
            if s.startswith("{"):
                enter_module(start, n, s[1:].strip(), braces, is_test)
                continue
        while s.startswith("#"):
            split = split_attribute(s)
            if split is None:
                attr = s
                break
            item_attr, s = split
            if not item_attr.startswith("#!"):
                pending_test_cfg = pending_test_cfg or is_test_cfg(item_attr)
                opened = None
                if TEST_FN.match(item_attr):
                    out.append(f"test function at line {n}")
            elif opened and is_test_cfg(item_attr):
                test_module(opened[0], n, opened[1] + braces)
        if attr or not s or depth or head:
            continue  # an attribute goes on, or a blank or comment line: a pending cfg still applies
        opened = None
        m = INLINE.match(s)
        if m:
            is_test = pending_test_cfg or m.group(4) == "tests"
            if m.group(5):
                enter_module(n, n, s[m.end() :].strip(), braces, is_test)
            else:
                head = (n, is_test)
        pending_test_cfg = False
    if code > MAX_LINES:
        out.append(f"{code} non-test lines (max {MAX_LINES})")
    return out


def problems(path: pathlib.Path) -> list[str]:
    return problems_in(path.read_text(encoding="utf-8").splitlines())


def entries(text: str) -> dict[str, str]:
    """The allowlist's entries (path -> line); `ValueError` for an entry
    without a reason."""
    out = {}
    for raw in text.splitlines():
        entry = raw.split("#", 1)[0].strip()
        if entry:
            if "#" not in raw or not raw.split("#", 1)[1].strip():
                raise ValueError(f"{entry}: no reason given")
            out[entry] = raw
    return out


def grown(base: dict[str, str], now: dict[str, str]) -> list[str]:
    """The entries `now` has and `base` does not."""
    return sorted(set(now) - set(base))


# (case, source, expected to be flagged as an inline test module)
SELF_TEST = [
    ("a cfg(test) inline", "#[cfg(test)]\nmod tests {\n}", True),
    ("b no space before brace", "#[cfg(test)]\nmod tests{\n}", True),
    ("c pub(crate)", "#[cfg(test)]\npub(crate) mod tests {\n}", True),
    ("d attribute between", "#[cfg(test)]\n#[allow(dead_code)]\nmod tests {\n}", True),
    ("e cfg(all(test, unix))", "#[cfg(all(test, unix))]\nmod procfs {\n}", True),
    ("f cfg(any(test, feature))", '#[cfg(any(test, feature = "x"))]\nmod helpers {\n}', True),
    ("g comment and blank between", "#[cfg(test)]\n// why\n\nmod tests {\n}", True),
    ("h bare mod tests", "mod tests {\n    #[test]\n    fn t() {}\n}", True),
    ("i file module", "#[cfg(test)]\nmod tests;", False),
    ("j other inline module", "mod inner {\n}", False),
    ("k cfg(test) item, then a module", "#[cfg(test)]\nfn helper() {}\nmod inner {\n}", False),
    ("l a feature named testing", '#[cfg(feature = "testing")]\nmod inner {\n}', False),
    ("m cfg wrapped by rustfmt", "#[cfg(all(\n    test,\n    unix\n))]\nmod procfs {\n}", True),
    ("n attribute wrapped, then cfg", "#[allow(\n    dead_code\n)]\n#[cfg(test)]\nmod helpers {\n}", True),
    ("o block comment between", "#[cfg(test)]\n/* why */\nmod helpers {\n}", True),
    ("p multi-line block comment", "#[cfg(test)]\n/*\n * why\n */\nmod helpers {\n}", True),
    ("q block comment before mod", "#[cfg(test)]\n/* why */ mod helpers {\n}", True),
    ("r cfg and mod on one line", "#[cfg(test)] mod helpers {\n}", True),
    ("s inner cfg(test)", "mod helpers {\n    #![cfg(test)]\n    fn h() {}\n}", True),
    (
        "t inner cfg wrapped, after a comment and an attribute",
        "mod helpers {\n    // why\n    #![allow(x)]\n    #![cfg(all(\n        test,\n        unix\n    ))]\n}",
        True,
    ),
    ("u inner cfg on the mod line", "mod helpers { #![cfg(test)]\n}", True),
    ("v inner cfg not first", "mod helpers {\n    fn h() {}\n    #![cfg(test)]\n}", False),
    ("w test in a comment", "#[cfg(unix)] // not test\nmod inner {\n}", False),
    ("x test in a string", '#[cfg(feature = "test")]\nmod inner {\n}', False),
    ("y cfg(test) in a block comment", "/* #[cfg(test)] */\nmod inner {\n}", False),
    ("z a lifetime is not a char", "#[cfg(test)]\nfn f<'a>(x: &'a str) {}\nmod inner {\n}", False),
    ("not(test) is not a test module", "#[cfg(not(test))]\nmod real {\n}", False),
    ("not(all(test, unix)) neither", "#[cfg(not(all(test, unix)))]\nmod real {\n}", False),
    ("all(unix, not(test)) neither", "#[cfg(all(unix, not(test)))]\nmod real {\n}", False),
    ("any(test, not(unix)) is", "#[cfg(any(test, not(unix)))]\nmod helpers {\n}", True),
    ("inner not(test) is not", "mod real {\n    #![cfg(not(test))]\n}", False),
    ("brace on the next line", "#[cfg(test)]\n#[rustfmt::skip]\nmod helpers\n{\n}", True),
    ("bare tests, brace on the next line", "mod tests\n{\n}", True),
    ("inner cfg, brace on the next line", "mod helpers\n{\n    #![cfg(test)]\n}", True),
    ("mod r#tests", "mod r#tests {\n}", True),
    ("cfg(test) mod r#x", "#[cfg(test)]\nmod r#helpers {\n}", True),
    ("no brace: a file module", "mod helpers\n;", False),
]

# (case, lines, expected problems): where a test module ends, and what counts
_LONG = ["// x"] * MAX_LINES
SELF_TEST_LINES = [
    (
        "count",
        _LONG + ["#[cfg(test)]", "mod tests {"] + ["    // t"] * 50 + ["}"],
        ["inline test module at line 302", f"{MAX_LINES + 1} non-test lines (max {MAX_LINES})"],
    ),
    (
        "braces in literals and comments",
        ["mod tests {", '    let s = "}";', "    let c = '}';", '    let r = r#"}"#;', "    // }", "    /* } */", "}"] + _LONG,
        ["inline test module at line 1"],
    ),
    (
        "an inner-cfg module's lines are test lines",
        ["mod helpers {", "    #![cfg(test)]"] + ["    // t"] * MAX_LINES + ["}"],
        ["inline test module at line 1"],
    ),
    ("a test function", ["#[test]", "fn t() {}"], ["test function at line 1"]),
    ("a tokio test", ["#[tokio::test(flavor = \"current_thread\")]", "async fn t() {}"], ["test function at line 1"]),
    ("a wrapped test attribute", ["#[tokio::test(", "    flavor = \"x\"", ")]", "async fn t() {}"], ["test function at line 3"]),
    ("cfg(test) and other attributes are not tests", ["#[cfg(test)]", "fn h() {}", "#[test_case]", "#[cfg_attr(test, derive(Debug))]", "struct S;"], []),
    ("a test inside a test module counts once", ["mod tests {", "    #[test]", "    fn t() {}", "}"], ["inline test module at line 1"]),
    (
        "an escaped quote and a multi-line string",
        ["#[cfg(test)]", 'const A: &str = "a\\"', "mod tests { }", '";', "mod inner {", "}"],
        [],
    ),
]


def self_test() -> int:
    failed = 0
    for name, src, want in SELF_TEST:
        got = any("inline" in p for p in problems_in(src.split("\n")))
        if got != want:
            print(f"self-test {name}: flagged={got}, want {want}")
            failed += 1
    for name, lines, want in SELF_TEST_LINES:
        if problems_in(lines) != want:
            print(f"self-test {name}: {problems_in(lines)}, want {want}")
            failed += 1
    base = entries("a.rs  # old\nb.rs  # old\n")
    for name, now, want in [
        ("shrunk", "a.rs  # old\n", []),
        ("same, reworded", "b.rs  # old\na.rs  # old, reworded\n", []),
        ("grown", "a.rs  # old\nb.rs  # old\nc.rs  # new\n", ["c.rs"]),
        ("swapped", "a.rs  # old\nc.rs  # new\n", ["c.rs"]),
    ]:
        if grown(base, entries(now)) != want:
            print(f"self-test allowlist {name}: {grown(base, entries(now))}, want {want}")
            failed += 1
    print("self-test ok" if not failed else f"self-test: {failed} failed")
    return 1 if failed else 0


def base_entries(ref: str) -> dict[str, str]:
    """The allowlist's entries at `ref`; none when `ref` has no allowlist yet
    (the PR that adds the check), so a first list must be empty to pass."""
    listed = subprocess.run(
        ["git", "-C", str(ROOT), "ls-tree", "--name-only", ref, "--", ALLOWLIST_REL],
        capture_output=True,
        check=False,
        text=True,
    )
    if listed.returncode == 0 and not listed.stdout.strip():
        return {}
    shown = subprocess.run(
        ["git", "-C", str(ROOT), "show", f"{ref}:{ALLOWLIST_REL}"],
        capture_output=True,
        check=False,
        text=True,
    )
    if shown.returncode != 0:
        raise SystemExit(f"cannot read {ALLOWLIST_REL} at {ref}: {shown.stderr.strip()}")
    return entries(shown.stdout)


def main(base: str | None) -> int:
    try:
        allowed = entries(ALLOWLIST.read_text(encoding="utf-8"))
        grew = grown(base_entries(base), allowed) if base is not None else []
    except ValueError as e:
        print(f"{ALLOWLIST.name}: {e}")
        return 1
    failed = False
    for key in grew:
        print(f"{key}: new in {ALLOWLIST.name} (not on {base}); the list may only shrink")
        failed = True
    seen = set()
    for path in rust_sources():
        rel = path.relative_to(ROOT)
        if is_test_file(rel):
            continue
        found = problems(path)
        key = rel.as_posix()
        if key in allowed:
            seen.add(key)
            if not found:
                print(f"{key}: complies now; remove it from {ALLOWLIST.name}")
                failed = True
            continue
        for p in found:
            print(f"{key}: {p}")
            failed = True
    for key in sorted(set(allowed) - seen):
        print(f"{key}: in {ALLOWLIST.name} but not a checked file; remove it")
        failed = True
    if not failed:
        print(f"structure ok ({len(allowed)} allowlisted)")
    return 1 if failed else 0


def rust_sources() -> list[pathlib.Path]:
    """Every Rust source file of the binary (`src/`) and the workspace crates
    (`crates/*/src/`); their `tests/` directories are tests, not sources."""
    roots = [ROOT / "src", *sorted((ROOT / "crates").glob("*/src"))]
    return sorted(p for root in roots for p in root.rglob("*.rs"))


def is_test_file(rel: pathlib.PurePath) -> bool:
    return rel.name == "tests.rs" or "tests" in rel.parts[1:-1]


if __name__ == "__main__":
    args = sys.argv[1:]
    if args == ["--self-test"]:
        sys.exit(self_test())
    if args and (len(args) != 2 or args[0] != "--base"):
        sys.exit("usage: " + __doc__.rsplit("usage: ", 1)[1].strip())
    sys.exit(main(args[1] if args else None))
