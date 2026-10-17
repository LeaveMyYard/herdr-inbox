"""The rules both plugins must share (herdr-panorama docs/plans/rust-rewrite.md §1.3 rows 3-5,
D6), as the same tables as herdr-panorama's tests/test_bridge_rules.py (herdr-panorama#53,
52275f4), case for case. Only the calls differ: panorama's discovery.is_uuid /
canonical_session_id / Session / hooks.is_claude are transcript.session_id_or_none /
transcript.continued_in / proc.is_claude here.

- one session-id validator (``transcript.session_id_or_none``) and its canonical form;
- the continuation rule (D6): a ``continued-in`` record of the main transcript (not a sidechain)
  naming another session id (compared in canonical lowercase) is cleared only by a later
  ``user`` or ``assistant`` record, never by bookkeeping records, known or new;
- "is this process Claude Code?" (``proc.is_claude``).

Every transcript record is a captured template (tests/fixtures/templates.json, bridge_scenario.py).
"""

import json

import pytest

from herdr_inbox import deeplink, proc, transcript

from .bridge_scenario import BASE, Project, iso, tpl

SID = "55555555-5555-4555-8555-555555555555"
NEXT = "66666666-6666-4666-8666-666666666666"
HEX = "abcdef01-2345-4abc-8def-0123456789ab"  # has letters: HEX.upper() != HEX

# -- one session-id validator --------------------------------------------------------------

VALID = [
    (SID, SID),
    ("0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d", "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"),
    ("0A1B2C3D-4E5F-4A6B-8C7D-9E0F1A2B3C4D", "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"),  # any case, canonical lower
    ("0a1B2c3D-4e5F-4a6B-8c7D-9e0F1a2B3c4D", "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"),
]

INVALID = [
    "",
    " " + SID,
    SID + " ",
    SID + "\n",  # the whole string, no trailing newline
    "\t" + SID,
    "5555555-5555-4555-8555-5555555555555",  # 7-4-4-4-13
    "55555555-5555-4555-8555-55555555555",  # short
    "55555555-5555-4555-8555-5555555555555",  # long
    "555555555555455585555555555555555",  # no dashes
    "55555555_5555_4555_8555_555555555555",
    "g5555555-5555-4555-8555-555555555555",  # not hex
    "../" + SID,
    SID + "/..",
    "..",
    "../../outside",
    "/etc/passwd",
    "a/b",
    "a\\b",
    "55555555-5555-4555-8555-55555555555/",
    "55555555-5555-4555-8555-55555555555\\",
    "*",
    "5555555*-5555-4555-8555-555555555555",
    "55555555-5555-4555-8555-55555555555?",
    "[5]555555-5555-4555-8555-555555555555",
    "agent-a1",
    None,
    123,
]


@pytest.mark.parametrize("sid,canonical", VALID)
def test_a_session_id_in_any_case_is_valid_and_canonical_is_lowercase(sid, canonical):
    assert transcript.session_id_or_none(sid) == canonical
    assert deeplink.valid("session", sid)


@pytest.mark.parametrize("sid", INVALID)
def test_anything_else_is_not_a_session_id(sid):
    assert transcript.session_id_or_none(sid) is None
    assert not deeplink.valid("session", sid)


def test_there_is_no_other_session_id_validator():
    """One validator (the drift in §1.3 row 4 was four regexes; here `_SESSION_CHARS`, a
    lowercase-only `_UUID` and `_TARGET`, and deeplink's `\\S+`)."""
    assert not hasattr(transcript, "_SESSION_CHARS")
    assert "session" not in deeplink.VALID


@pytest.mark.parametrize("sid", [SID, HEX])
def test_an_uppercase_id_finds_its_lowercase_transcript(tmp_path, monkeypatch, sid):
    """Claude Code names transcripts by the lowercase id; an id handed over in another case is
    the same session."""
    assert HEX.upper() != HEX
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path))
    proj = Project(tmp_path / "projects")
    m = proj.main(sid)
    m.text(BASE, "hi")
    m.write()
    assert transcript.find(sid.upper()) == m.path


def test_a_continuation_named_in_uppercase_is_found_and_followed(tmp_path, monkeypatch):
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(tmp_path))
    proj = Project(tmp_path / "projects")
    old = old_session(tmp_path)
    continued(old, BASE + 10, HEX.upper())
    old.write()
    new = proj.main(HEX)
    new.prompt(BASE + 11, "next")
    new.write()
    ids = transcript.chain(SID)
    assert ids == [SID, HEX] and transcript.find(ids[-1]) == new.path
    assert transcript.find(HEX.upper()) == new.path  # herdr naming the new session in uppercase


# -- the continuation rule (D6) ------------------------------------------------------------

UNKNOWN = "made-up-bookkeeping"  # a record type Claude Code may add tomorrow


def bookkeeping(m, kind: str) -> None:
    """A captured bookkeeping record, as Claude Code writes it (undated); ``UNKNOWN``: the
    captured ``mode`` record under a type this code has never seen."""
    rec = tpl("mode" if kind == UNKNOWN else kind)
    if kind == UNKNOWN:
        rec["type"] = UNKNOWN
    rec["sessionId"] = m.session
    m.raw(json.dumps(rec, separators=(",", ":")))


def continued(m, t: float, nxt: str = NEXT) -> None:
    rec = tpl("continued_in")
    rec.update(timestamp=iso(t), sessionId=m.session, continuedInSessionId=nxt)
    m.raw(json.dumps(rec, separators=(",", ":")))


class Loaded:
    """panorama's Session, as far as these tables look at it: ``continued_in`` after a read,
    ``update()`` to read what was appended since."""

    def __init__(self, m):
        self.path = m.path
        self.update()

    def update(self) -> None:
        self.continued_in = transcript.continued_in(self.path)


def load(m) -> Loaded:
    return Loaded(m)


def old_session(tmp_path):
    proj = Project(tmp_path / "projects")
    m = proj.main(SID)
    m.prompt(BASE, "work")
    m.text(BASE + 1, "working")
    return m


@pytest.mark.parametrize("kinds", [
    [], ["last_prompt"], ["cost_state"], ["mode"], [UNKNOWN],
    ["last_prompt", "cost_state", "mode", UNKNOWN],
])
def test_bookkeeping_after_continued_in_does_not_clear_it(tmp_path, kinds):
    m = old_session(tmp_path)
    continued(m, BASE + 10)
    for kind in kinds:
        bookkeeping(m, kind)
    m.write()
    assert load(m).continued_in == NEXT


@pytest.mark.parametrize("kind", ["user", "assistant"])
@pytest.mark.parametrize("dt,cleared", [(+1, True), (0, False), (-1, False)])
def test_only_a_later_conversation_record_clears_it(tmp_path, kind, dt, cleared):
    """``claude --resume <old>``: the conversation goes on in the old file, so it is live again.
    A conversation record at or before the continued-in's time is not a resume."""
    m = old_session(tmp_path)
    continued(m, BASE + 10)
    bookkeeping(m, "last_prompt")
    bookkeeping(m, "cost_state")
    (m.prompt if kind == "user" else m.text)(BASE + 10 + dt, "go on")
    m.write()
    s = load(m)
    assert (s.continued_in is None) == cleared
    assert s.continued_in in (None, NEXT)


def test_a_resume_appended_later_clears_it_on_the_next_read(tmp_path):
    m = old_session(tmp_path)
    continued(m, BASE + 10)
    bookkeeping(m, "mode")
    bookkeeping(m, UNKNOWN)
    m.write()
    s = load(m)
    assert s.continued_in == NEXT
    m.prompt(BASE + 20, "go on")
    m.write()
    s.update()
    assert s.continued_in is None


@pytest.mark.parametrize("nxt", ["../../outside", "*", "/etc/passwd", "", SID, SID.upper(), NEXT + "\n"])
def test_a_continuation_that_is_not_another_session_id_is_ignored(tmp_path, nxt):
    """Not a UUID, or this session's own id (in any case): never followed."""
    m = old_session(tmp_path)
    continued(m, BASE + 10, nxt)
    m.write()
    assert load(m).continued_in is None


def test_a_continuation_in_another_case_is_kept_canonical(tmp_path):
    m = old_session(tmp_path)
    continued(m, BASE + 10, HEX.upper())
    m.write()
    assert load(m).continued_in == HEX


@pytest.mark.parametrize("own,nxt", [(HEX, HEX.upper()), (HEX.upper(), HEX), (HEX, HEX.title())])
def test_a_continuation_to_its_own_id_in_another_case_is_ignored(tmp_path, own, nxt):
    proj = Project(tmp_path / "projects")
    m = proj.main(own)
    m.prompt(BASE, "work")
    continued(m, BASE + 10, nxt)
    m.write()
    assert load(m).continued_in is None


def test_a_sidechain_continuation_is_ignored(tmp_path):
    """A continued-in record marked ``isSidechain: true`` is never the main conversation's, even
    in the main transcript."""
    m = old_session(tmp_path)
    rec = tpl("continued_in")
    rec.update(timestamp=iso(BASE + 10), sessionId=SID, isSidechain=True, continuedInSessionId=NEXT)
    m.raw(json.dumps(rec, separators=(",", ":")))
    m.write()
    assert load(m).continued_in is None


@pytest.mark.parametrize("ignored", ["sidechain", "own_id", "not_a_uuid"])
def test_a_resume_before_an_ignored_continuation_in_the_same_batch_still_counts(tmp_path, ignored):
    """continued-in, then the resume, then a continued-in that is ignored, all in one read: the
    resume scan starts after the last ACCEPTED continued-in, so the resume still clears it."""
    m = old_session(tmp_path)
    continued(m, BASE + 10)
    m.prompt(BASE + 20, "go on")
    rec = tpl("continued_in")
    rec.update(timestamp=iso(BASE + 30), sessionId=SID,
               continuedInSessionId={"own_id": SID.upper(), "not_a_uuid": "../x"}.get(ignored, NEXT))
    if ignored == "sidechain":
        rec["isSidechain"] = True
    m.raw(json.dumps(rec, separators=(",", ":")))
    m.write()
    assert load(m).continued_in is None


def undated_continued(m, nxt: str = NEXT) -> None:
    """A continued-in record without a timestamp: it takes the time of the record before it."""
    rec = tpl("continued_in")
    rec.pop("timestamp")
    rec.update(sessionId=m.session, continuedInSessionId=nxt)
    m.raw(json.dumps(rec, separators=(",", ":")))


@pytest.mark.parametrize("kind", ["user", "assistant"])
@pytest.mark.parametrize("dt,cleared", [(+1, True), (0, False), (-1, False)])
def test_an_undated_continuation_takes_the_previous_dated_records_time(tmp_path, kind, dt, cleared):
    """Its time is that of the last DATED record of any type before it (here the assistant text
    at BASE + 5, a record the scanner's prefilter does not parse); undated bookkeeping between
    them does not move it."""
    proj = Project(tmp_path / "projects")
    m = proj.main(SID)
    m.prompt(BASE, "work")
    m.prompt(BASE + 1, "more")
    m.text(BASE + 5, "working")
    bookkeeping(m, "mode")
    undated_continued(m)
    bookkeeping(m, "cost_state")
    (m.prompt if kind == "user" else m.text)(BASE + 5 + dt, "go on")
    m.write()
    assert (load(m).continued_in is None) == cleared


@pytest.mark.parametrize("dt,cleared", [(+1, True), (0, False)])
def test_an_undated_continuation_takes_a_dated_record_from_an_earlier_read(tmp_path, dt, cleared):
    """The dated record was read in an earlier batch than the undated continued-in."""
    proj = Project(tmp_path / "projects")
    m = proj.main(SID)
    m.prompt(BASE, "work")
    m.text(BASE + 5, "working")
    m.write()
    s = load(m)
    undated_continued(m)
    m.write()
    s.update()
    assert s.continued_in == NEXT
    m.text(BASE + 5 + dt, "go on")
    m.write()
    s.update()
    assert (s.continued_in is None) == cleared


def test_an_undated_continuation_with_no_time_before_it_is_cleared_by_any_conversation(tmp_path):
    """Nothing dated before it: no time to compare with, so any later user/assistant record is a
    resume; bookkeeping still is not."""
    proj = Project(tmp_path / "projects")
    m = proj.main(SID)
    undated_continued(m)
    bookkeeping(m, "last_prompt")
    bookkeeping(m, UNKNOWN)
    m.write()
    s = load(m)
    assert s.continued_in == NEXT
    m.text(BASE, "go on")
    m.write()
    s.update()
    assert s.continued_in is None


def test_a_continuation_from_a_subagent_file_is_ignored(tmp_path):
    """Only the main transcript's continued-in counts."""
    proj = Project(tmp_path / "projects")
    for t in (proj.agent(SID, "a1"), proj.main(SID)):
        t.prompt(BASE, "work")
        continued(t, BASE + 10)
        t.write()
    assert transcript.continued_in(proj.agent(SID, "a1").path) is None
    assert transcript.continued_in(proj.main(SID).path) == NEXT


# -- is this process Claude Code? ----------------------------------------------------------

CLAUDE = [
    ("claude", []),  # native install, by command name
    ("claude", ["claude", "--resume"]),
    ("2.1.301", ["/home/u/.local/share/claude/versions/2.1.301"]),  # versioned binary: comm is the version
    ("2.1.301", ["/home/u/.local/share/claude/versions/2.1.301", "--continue"]),
    ("MainThread", ["/home/u/.local/bin/claude"]),  # a symlink named claude
    ("node", ["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"]),
    ("node", ["/usr/bin/node", "/home/u/.npm-global/lib/node_modules/@anthropic-ai/claude-code/cli.js", "-p"]),
    ("node", ["node", "--no-warnings", "/opt/x/@anthropic-ai/claude-code/cli.js"]),  # one option before it
    ("node", ["/home/u/.nvm/versions/node/v22.0.0/bin/node", "/home/u/src/claude-code/cli.js"]),
    ("node22", ["node22", "/x/@anthropic-ai/claude-code/cli.mjs"]),
]

NOT_CLAUDE = [
    ("bash", ["bash", "-c", "claude"]),  # a shell that will start claude is not claude
    ("python3", ["python3", "-m", "claude"]),
    ("claude-code", ["claude-code"]),
    ("claudette", ["/usr/bin/claudette"]),
    ("node", ["node", "server.js"]),
    ("node", ["node", "a", "b", "/x/@anthropic-ai/claude-code/cli.js"]),  # only the first two args
    ("node", []),
    ("uv", ["uv", "run", "/x/@anthropic-ai/claude-code/cli.js"]),  # not node
    ("zsh", ["/home/u/.local/share/claude-versions/2.1.301"]),
    ("", []),
]


@pytest.mark.parametrize("comm,argv", CLAUDE)
def test_is_claude(comm, argv):
    assert proc.is_claude(comm, argv)


@pytest.mark.parametrize("comm,argv", NOT_CLAUDE)
def test_is_not_claude(comm, argv):
    assert not proc.is_claude(comm, argv)
