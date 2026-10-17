"""Inbox-side checks of the shared rules (tests/test_bridge_rules.py) where the inbox reads
them: its process walk (proc.nested_claude / owning_claude), the incremental continued_in, and
the real shapes of continued-in transcripts captured on 2.1.301."""

import json

import pytest

from herdr_inbox import proc, transcript

from .bridge_scenario import BASE, Project, iso, tpl
from .test_bridge_rules import NEXT, SID, bookkeeping, continued

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


def test_an_uppercase_id_with_letters_finds_and_follows_its_lowercase_transcript(tmp_path, monkeypatch):
    """The shared case uses an all-digit id (upper() == lower()); this one has hex letters."""
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path))
    m = Project(tmp_path / "projects").main(LETTERS)
    m.prompt(BASE, "work")
    continued(m, BASE + 10)
    m.write()
    assert transcript.find(LETTERS.upper()) == m.path
    assert transcript.follow(LETTERS.upper()) == NEXT
