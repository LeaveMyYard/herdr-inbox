"""Inbox-side checks of the shared rules (tests/test_bridge_rules.py) where the inbox reads
them: its process walk (proc.nested_claude / owning_claude), the incremental continued_in, and
the real shapes of continued-in transcripts captured on 2.1.301."""

import json
from pathlib import Path

import pytest

from herdr_inbox import proc, transcript

from .bridge_scenario import BASE, Project, tpl
from .test_bridge_rules import INVALID, NEXT, SID, bookkeeping, continued

VERSIONED = ("2.1.301", ["/home/u/.local/share/claude/versions/2.1.301"])
NODE = ("node", ["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"])
NATIVE = ("claude", ["claude"])
LETTERS = "abcdef01-2345-4abc-8def-0123456789ab"
SHELL = ("zsh", ["zsh"])


def tree(monkeypatch, chain):
    """A fake process tree above us: ``chain`` is [(comm, argv), ...] from our parent up."""
    procs = {100 + i: (101 + i, comm, argv) for i, (comm, argv) in enumerate(chain)}
    monkeypatch.setattr(proc.os, "getppid", lambda: 100)
    monkeypatch.setattr(proc, "_parent", lambda pid: (procs[pid][0], procs[pid][1]) if pid in procs else None)
    monkeypatch.setattr(proc, "_cmdline", lambda pid: procs[pid][2] if pid in procs else [])


@pytest.mark.parametrize("inner,outer", [(VERSIONED, NATIVE), (NATIVE, VERSIONED), (NODE, NODE), (VERSIONED, VERSIONED)])
def test_a_nested_claude_is_found_whatever_the_install(monkeypatch, inner, outer):
    """§1.3 row 5: `comm == "claude"` missed versioned binaries and node installs."""
    tree(monkeypatch, [SHELL, inner, SHELL, SHELL, outer, SHELL])
    assert proc.nested_claude()
    assert proc.owning_claude() == 101  # the nearest Claude, not the shell above us


@pytest.mark.parametrize("only", [VERSIONED, NODE, NATIVE])
def test_one_claude_of_any_install_is_not_nested(monkeypatch, only):
    tree(monkeypatch, [SHELL, only, SHELL])
    assert not proc.nested_claude()
    assert proc.owning_claude() == 101


def test_no_claude_above_falls_back_to_the_grandparent(monkeypatch):
    tree(monkeypatch, [SHELL, ("bash", ["bash", "-c", "claude"]), SHELL])
    assert not proc.nested_claude()
    assert proc.owning_claude() == 101


def test_the_argv_is_read_only_for_a_process_not_named_claude(monkeypatch):
    tree(monkeypatch, [NATIVE, SHELL])
    monkeypatch.setattr(proc, "_cmdline", lambda pid: pytest.fail("argv read for a process named claude"))
    assert proc.owning_claude() == 100


# -- continued_in as the inbox reads it: incremental, cached ------------------------------------

def test_a_resume_in_a_later_read_clears_and_bookkeeping_in_a_later_read_does_not(tmp_path):
    """The incremental read keeps the continued-in's time between reads (review #41's cache)."""
    m = Project(tmp_path / "projects").main(SID)
    m.prompt(BASE, "work")
    continued(m, BASE + 10)
    m.write()
    assert transcript.continued_in(m.path) == NEXT
    for kind in ("last_prompt", "cost_state", "mode"):
        bookkeeping(m, kind)
        m.write()
        assert transcript.continued_in(m.path) == NEXT
    m.prompt(BASE + 10, "same second: not a resume")
    m.write()
    assert transcript.continued_in(m.path) == NEXT
    m.text(BASE + 11, "resumed")
    m.write()
    assert transcript.continued_in(m.path) is None


def test_an_undated_continuation_keeps_the_last_time_across_reads(tmp_path):
    """The time of the last dated record survives the read boundary too."""
    m = Project(tmp_path / "projects").main(SID)
    m.prompt(BASE + 5, "work")
    m.write()
    assert transcript.continued_in(m.path) is None
    rec = tpl("continued_in")
    rec.pop("timestamp")
    rec.update(sessionId=SID, continuedInSessionId=NEXT)
    m.raw(json.dumps(rec, separators=(",", ":")))
    m.write()
    assert transcript.continued_in(m.path) == NEXT
    m.prompt(BASE + 5, "not later")
    m.write()
    assert transcript.continued_in(m.path) == NEXT
    m.prompt(BASE + 6, "later")
    m.write()
    assert transcript.continued_in(m.path) is None


def test_an_undated_resume_never_clears_a_dated_continuation(tmp_path):
    m = Project(tmp_path / "projects").main(SID)
    continued(m, BASE + 10)
    rec = tpl("user_prompt_human")
    rec.update(sessionId=SID)
    rec.pop("timestamp", None)
    m.raw(json.dumps(rec, separators=(",", ":")))
    m.write()
    assert transcript.continued_in(m.path) == NEXT


# The order of record types after each continued-in found in this machine's transcripts
# (2.1.301, 2026-10-15..17; record bodies reduced to type and timestamp, the only fields the rule reads): two
# continued and left, two resumed later with `claude --resume`. file-history-snapshot was not on
# the old 7-type bookkeeping list, so the old rule stopped following at it, before the resume.
CAPTURED = [
    ([("continued-in", "2026-10-15T08:09:43.909Z"), ("last-prompt", None), ("cost-state", None)], True),
    ([("continued-in", "2026-10-16T11:22:22.458Z"), ("cost-state", None), ("file-history-snapshot", None)], True),
    ([("continued-in", "2026-10-16T11:22:22.458Z"), ("cost-state", None), ("file-history-snapshot", None),
      ("user", "2026-10-17T06:32:56.399Z"), ("api-request-shape", "2026-10-17T06:32:57.085Z")], False),
    ([("continued-in", "2026-10-16T18:41:47.545Z"), ("last-prompt", None), ("cost-state", None),
      ("file-history-snapshot", None), ("assistant", "2026-10-17T06:32:48.894Z"), ("api-request-blob", None)], False),
]


@pytest.mark.parametrize("types,continued_still", CAPTURED)
def test_the_captured_orders(tmp_path, types, continued_still):
    templates = {"continued-in": "continued_in", "last-prompt": "last_prompt", "cost-state": "cost_state",
                 "user": "user_prompt_human", "assistant": "assistant_text"}
    m = Project(tmp_path / "projects").main(SID)
    for kind, ts in types:
        rec = tpl(templates[kind]) if kind in templates else {"type": kind}  # unlisted: type only
        rec["sessionId"] = SID
        rec.pop("timestamp", None)
        if ts:
            rec["timestamp"] = ts
        if kind == "continued-in":
            rec["continuedInSessionId"] = NEXT
        m.raw(json.dumps(rec, separators=(",", ":")))
    m.write()
    assert (transcript.continued_in(m.path) == NEXT) == continued_still


def test_continued_targets_go_through_the_one_validator(tmp_path):
    """Every target ever named, canonical lowercase; anything that is not a session id dropped."""
    m = Project(tmp_path / "projects").main(SID)
    for nxt in (NEXT.upper(), "../../outside", "*", SID + " "):
        continued(m, BASE, nxt)
    m.write()
    assert transcript.continued_targets(m.path) == frozenset({NEXT})


def test_a_session_deep_link_matches_in_any_case(store):
    from herdr_inbox import deeplink, service
    from herdr_inbox.service import Caller

    qid = service.ask(store, Caller("w1:p1", "w1:t1", "tab", LETTERS.upper(), "/r"), "Linked?")["id"]
    group = store.get(qid).group
    assert deeplink.resolve(store, {"kind": "session", "value": LETTERS}) == (("group", group), None)
    assert deeplink.resolve(store, {"kind": "session", "value": LETTERS.upper()}) == (("group", group), None)
    assert deeplink.resolve(store, {"kind": "session", "value": "../" + LETTERS}) is None


@pytest.mark.parametrize("sid", INVALID)
def test_a_bad_session_id_is_never_used_as_a_path(tmp_path, monkeypatch, sid):
    """The id reaches find from herdr, the env, hook payloads and transcripts: anything but a
    canonical UUID never reaches the file system (no glob, no stat, no open)."""
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path))
    proj = Project(tmp_path / "projects")
    proj.main(SID).write()
    touched = []
    for name in ("glob", "is_file", "exists", "stat"):
        real = getattr(Path, name)
        monkeypatch.setattr(Path, name, lambda self, *a, _real=real, **k: touched.append(str(self)) or _real(self, *a, **k))
    assert transcript.find(sid) is None
    assert transcript.chain(sid) in ([], [sid])  # itself only, never a continuation read
    assert touched == []


def test_bookkeeping_over_64_kib_between_reads_keeps_the_mark(tmp_path):
    """Review #47 L1: a growth larger than the cold-read window is read whole, so D6's 'bookkeeping
    never clears it' holds however much of it arrives between two looks."""
    m = Project(tmp_path / "projects").main(SID)
    m.prompt(BASE, "work")
    continued(m, BASE + 10)
    m.write()
    assert transcript.continued_in(m.path) == NEXT
    while sum(len(x) + 1 for x in m.lines) < 3 * transcript.FOLLOW_TAIL:
        bookkeeping(m, "cost_state")
    m.write()
    assert transcript.continued_in(m.path) == NEXT
    m.prompt(BASE + 11, "resumed")
    m.write()
    assert transcript.continued_in(m.path) is None


def test_chain_and_follow_return_the_canonical_id(tmp_path, monkeypatch):
    """Review #47 L3: an id handed over in another case comes back canonical; anything that is
    not a session id comes back as it was (never looked up)."""
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path))
    hex_id = "abcdef01-2345-4abc-8def-0123456789ab"
    Project(tmp_path / "projects").main(hex_id).write()
    assert transcript.chain(hex_id.upper()) == [hex_id]
    assert transcript.follow(hex_id.upper()) == hex_id
    assert transcript.follow("not-a-session") == "not-a-session"


@pytest.mark.parametrize("bad", ["../../x", "*", "a/b", SID + " ", "agent-a1"])
def test_rewake_never_names_a_file_after_a_bad_session_id(store, fake_herdr, monkeypatch, bad):
    """Review #47 L2: the watcher's lock and owner files are named only by the one validator."""
    from herdr_inbox import rewake

    monkeypatch.setenv("HERDR_PANE_ID", "w1:p1")
    before = set(rewake._dir().iterdir())
    assert rewake.run({"session_id": bad}) is None
    assert not rewake.watcher_alive(bad)
    assert set(rewake._dir().iterdir()) == before
    with pytest.raises(ValueError):
        rewake._safe(bad)


def test_rewake_files_use_the_canonical_id(store, fake_herdr):
    from herdr_inbox import rewake

    hex_id = "abcdef01-2345-4abc-8def-0123456789ab"
    assert rewake._owner_path(hex_id.upper()).name == f"{hex_id}.owner"


def test_chain_keeps_the_known_path_for_an_id_in_another_case(tmp_path, monkeypatch):
    """Review #47 L5: the recorded transcript path (outside the projects dir here) belongs to the
    first session whatever the case of its id."""
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path / "elsewhere"))
    hex_id = "abcdef01-2345-4abc-8def-0123456789ab"
    m = Project(tmp_path / "projects").main(hex_id)
    m.prompt(BASE, "work")
    continued(m, BASE + 10)
    m.write()
    assert transcript.chain(hex_id.upper(), str(m.path)) == [hex_id, NEXT]


def test_a_workflow_agents_transcript_is_a_subagents_too(tmp_path):
    m = Project(tmp_path / "projects")
    wf = tmp_path / "projects" / "-home-user-project" / SID / "subagents" / "workflows" / "run1"
    wf.mkdir(parents=True)
    path = wf / "agent-a1.jsonl"
    rec = tpl("continued_in")
    rec.update(sessionId=SID, continuedInSessionId=NEXT)
    path.write_text(json.dumps(rec) + "\n")
    assert transcript.continued_in(path) is None
    assert m  # the project dir exists either way


def test_a_config_dir_under_a_folder_named_subagents_still_follows(tmp_path, monkeypatch):
    """Final pass L1: only the transcript's own place decides, never a folder above the projects dir."""
    root = tmp_path / "subagents" / "claude"
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(root))
    m = Project(root / "projects").main(SID)
    m.prompt(BASE, "work")
    continued(m, BASE + 10)
    m.write()
    assert transcript.chain(SID) == [SID, NEXT]


def test_a_transcript_that_shrank_is_read_again_from_its_tail(tmp_path):
    """Final pass L2: a rewritten (shorter) file is never read from the old cached offset."""
    m = Project(tmp_path / "projects").main(SID)
    m.prompt(BASE, "work")
    for _ in range(40):
        bookkeeping(m, "cost_state")
    m.write()
    assert transcript.continued_in(m.path) is None
    m.lines = m.lines[:1]
    continued(m, BASE + 10)
    m.write()
    assert transcript.continued_in(m.path) == NEXT


def test_a_bad_session_link_never_matches_questions_without_a_session(store):
    """Final pass N1: an invalid link canonicalises to None, as does a question with no session."""
    from herdr_inbox import deeplink, service
    from herdr_inbox.service import Caller

    service.ask(store, Caller("w1:p1", "w1:t1", "tab", None, "/r"), "No session?")
    assert deeplink.resolve(store, {"kind": "session", "value": "not-a-session"}) is None


def test_without_procfs_a_walk_reads_the_process_table_once(monkeypatch):
    """macOS: is_claude needs argv for every non-claude ancestor; asking ps per ancestor doubled
    the cost of every hook (CI: SessionStart over 1 s). Two ps calls per walk, whatever the depth."""
    import builtins
    import subprocess

    real_open = builtins.open

    def no_proc(path, *a, **k):
        if str(path).startswith("/proc/"):
            raise FileNotFoundError(path)
        return real_open(path, *a, **k)

    chain = [(100, 101, "python3", "python3 -I hooks.py"), (101, 102, "zsh", "zsh"),
             (102, 103, "2.1.301", "/Users/u/.local/share/claude/versions/2.1.301 -p"),
             (103, 104, "bash", "bash -c x"), (104, 105, "/Users/u/.local/bin/claude", "/Users/u/.local/bin/claude"),
             (105, 1, "/bin/zsh", "-zsh")]
    calls = []

    def fake_run(argv, **k):
        calls.append(argv)
        if "pid=,ppid=,comm=" in argv:
            out = "".join(f"{p:>6} {pp:>6} {comm}\n" for p, pp, comm, _ in chain)
        else:
            out = "".join(f"{p:>6} {cmd}\n" for p, _, _, cmd in chain)
        return subprocess.CompletedProcess(argv, 0, out, "")

    monkeypatch.setattr(builtins, "open", no_proc)
    monkeypatch.setattr(proc.subprocess, "run", fake_run)
    monkeypatch.setattr(proc.os, "getppid", lambda: 100)
    assert proc.nested_claude()
    assert len(calls) == 2
    assert proc.owning_claude() == 102
    assert len(calls) == 4  # a new walk reads a fresh table
    assert proc._parent(104) == (105, "claude") and proc._cmdline(102)[0].endswith("/2.1.301")
    assert len(calls) == 8  # outside a walk nothing is kept
