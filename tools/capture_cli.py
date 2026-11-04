"""The CLI golden, ``tests/fixtures/cli.json`` (``tools/capture_goldens.py --only cli``): Python's
``cli.main`` run in-process over the script ``CLI_OPS`` (see tests/fixtures/README.md).

Each ``cli`` step runs one command line and records its exit code, stdout, stderr, what it asked of
herdr, the delivery and the worker (``effects``), and the tables after it. herdr, the delivery pass
and the worker are stubbed at their function boundary (``herdr.set_open_count``, ``pane_tabs``,
``tab_label``, ``rename_tab``, ``notify``; ``service.deliver_batch``; ``worker.ensure`` and
``service._ensure_worker``): the delivery pass has goldens of its own (decisions.json,
store-delivery.json), and this one pins what the CLI does around it. A stubbed delivery writes the
step's outcome (``delivery``) onto the rows it is given. Steps can set the plugin's
``config.toml`` (``config``), write transcripts (``files``) and set the caller's
``CLAUDE_CODE_SESSION_ID`` / ``HERDR_PANE_ID`` (``me``). ``submit`` steps run ``service.submit``
without delivery (the popup's path), and ``preamble`` steps run the real ``deliver_batch`` with every
claim refused, which records only its preamble: the worker ensured and the answers it keeps
for this herdr server (one foreign server per step: Python ensures them in set order). In stderr
the work directory is ``<work>`` and a TOML parser's reason ``<reason>``. Times are relative to now (ages print as ``3h``) and masked in the tables.
"""

from __future__ import annotations

import contextlib
import copy
import io
import json
import logging
import os
import re
import sys
from pathlib import Path

S_A = "aaaaaaaa-0000-4000-8000-00000000000a"
S_OLD = "a58e3f68-0000-4000-8000-000000000001"
S_NEW = "06acc80f-0000-4000-8000-000000000002"
S_X = "77777777-0000-4000-8000-000000000007"


def _ago(spec: str) -> str:
    return f"strftime('%Y-%m-%dT%H:%M:%S+00:00', 'now', '{spec}')"


def _q(qid, ago, pane, question, status="open", tab_label=None, session=None, asked_by=None, answer=None,
       repeat=0, tab="w1:t1"):
    return ("sql", {"sql": "INSERT INTO questions (id, created_at, pane_id, tab_id, tab_label, session_id, cwd,"
                           " asked_by, question, norm, status, answer, answered_at, repeat_count)"
                           f" VALUES (?, {_ago(ago)}, ?, ?, ?, ?, '/r', ?, ?, ?, ?, ?, {_ago(ago)}, ?)",
                    "params": [qid, pane, tab if pane else None, tab_label, session, asked_by, question,
                               question.lower(), status, answer, repeat]})


def _draft(qid, detail):
    return ("sql", {"sql": "INSERT INTO question_ext (id, group_id, draft_json, drafted_at) VALUES (?, ?, ?, ?)"
                           " ON CONFLICT(id) DO UPDATE SET draft_json = excluded.draft_json",
                    "params": [qid, qid, detail if isinstance(detail, str) else json.dumps(detail),
                               "2001-01-01T00:00:00+00:00"]})


def _rec(kind, **kw):
    return {"type": kind, "timestamp": "2026-10-17T10:00:00.000Z", **kw}


ON = 'tab_prefix = "❓"\n'
TABS = [["w1:p1", "w1:t1"], ["w1:p2", "w1:t2"], ["w1:p3", "w1:t1"], ["w1:p9", None]]
LONG = ("Should the migration run before the release goes out, or after, given that the new schema "
        "drops two columns that the old workers still read?")
CLI_OPS = [
    # no database yet: the CLI never creates or migrates it; the worker does (and is started)
    ("cli", {"argv": ["list"]}),
    ("cli", {"argv": ["answer", "1", "go"]}),
    ("cli", {"argv": ["dismiss", "1"]}),
    ("migrate", {}),
    _q(1, "-3 hours", "w1:p1", "Which region?", tab_label="my-tab", session=S_A, asked_by="lead"),
    _q(2, "-5 minutes", "w1:p2", "Ship it?\nSecond line ignored", repeat=2, session=S_A, tab="w1:t2"),
    _q(3, "-2 days", None, "Old one?", status="answered", answer="yes, keep it"),
    _q(4, "-40 minutes", "w1:p3", "Dismissed?", status="dismissed", tab_label=""),
    _q(5, "-90 minutes", "w1:p3", LONG, asked_by="", session=S_X),
    _q(6, "-1 hours", "w1:p1", "Answered by its own conversation?", session=S_A),
    _q(7, "-1 hours", "w1:p1", "Answered by a continued conversation?", session=S_NEW),
    _q(8, "-1 hours", "w1:p1", "Answered from another agent?", session=S_NEW),
    ("cli", {"argv": ["list"]}),
    ("cli", {"argv": ["list", "--all"]}),
    ("cli", {"argv": ["list", "--al"]}),
    # usage errors and help, as argparse words them
    ("cli", {"argv": ["list", "-h"]}),
    ("cli", {"argv": ["list", "--he"]}),
    ("cli", {"argv": ["list", "-hx"]}),
    ("cli", {"argv": ["answer", "--help"]}),
    ("cli", {"argv": ["dismiss", "-h"]}),
    ("cli", {"argv": ["list", "extra"]}),
    ("cli", {"argv": ["list", "--bogus"]}),
    ("cli", {"argv": ["list", "--all=x"]}),
    ("cli", {"argv": ["list", "--h=1"]}),
    ("cli", {"argv": ["answer"]}),
    ("cli", {"argv": ["answer", "1"]}),
    ("cli", {"argv": ["answer", "x", "y"]}),
    ("cli", {"argv": ["answer", "QQ12", "go"]}),
    ("cli", {"argv": ["answer", "qnap:Q12", "go"]}),
    ("cli", {"argv": ["answer", "1", "go", "--x", "more"]}),
    ("cli", {"argv": ["answer", "x", "-h"]}),
    ("cli", {"argv": ["answer", "1", "a", "-h"]}),
    ("cli", {"argv": ["list", "--"]}),
    ("cli", {"argv": ["list", "--", "x"]}),
    ("cli", {"argv": ["list", "--all", "--"]}),
    ("cli", {"argv": ["dismiss", "--", "1", "2"]}),
    ("cli", {"argv": ["dismiss"]}),
    ("cli", {"argv": ["dismiss", "1", "2"]}),
    ("cli", {"argv": ["dismiss", "it's"]}),
    ("cli", {"argv": ["dismiss", "a\"b'c\td\x07é "]}),
    # ids as Python's int() reads them
    ("cli", {"argv": ["dismiss", " +9_9 "]}),
    ("cli", {"argv": ["dismiss", "q١٢"]}),
    ("cli", {"argv": ["dismiss", "Q-3"]}),
    ("cli", {"argv": ["dismiss", "1__0"]}),
    # answers: delivered, not delivered, not open, blank
    ("cli", {"argv": ["answer", "Q1", "go", "ahead"], "config": ON, "pane_tabs": TABS,
             "labels": {"w1:t1": "❓4 my-tab"}, "delivery": {"delivered": True}}),
    ("cli", {"argv": ["answer", "q5", "  spaced\tout  "], "config": ON, "pane_tabs": TABS,
             "labels": {"w1:t1": "❓3 my-tab"}, "delivery": {"error": "no live agent in pane w1:p3"}}),
    ("cli", {"argv": ["answer", "1", "again"], "delivery": {"delivered": True}}),
    ("cli", {"argv": ["answer", "99", "x"]}),
    ("cli", {"argv": ["answer", "2", "   "]}),
    ("cli", {"argv": ["answer", "--", "2", "--all", "-x"], "config": 'tab_prefix = ""\n', "pane_tabs": TABS,
             "delivery": {"delivered": True}}),
    # the asker answers itself: nothing typed, landed but not seen
    ("cli", {"argv": ["answer", "6", "decided", "in", "chat"], "me": {"session": S_A, "pane": "w1:p1"},
             "files": {S_A: [_rec("user", message={"role": "user", "content": "hi"})]},
             "pane_tabs": None, "delivery": {"delivered": True}}),
    # its older session, continued into the asker: the same conversation
    ("cli", {"argv": ["answer", "7", "relayed"], "me": {"session": S_OLD, "pane": "w1:p1"},
             "files": {S_OLD: [_rec("user", message={"role": "user", "content": "before"}),
                               {"type": "continued-in", "sessionId": S_OLD, "continuedInSessionId": S_NEW}],
                       S_NEW: [{"type": "ai-title", "aiTitle": "x"}]},
             "sessions": [S_OLD], "delivery": {"delivered": True}}),
    # another agent's process: delivered as any answer
    ("cli", {"argv": ["answer", "8", "from", "elsewhere"], "me": {"session": S_X, "pane": "w1:p2"},
             "delivery": {"delivered": True}}),
    # dismissals and the tab marks
    _q(10, "-1 hours", "w1:p2", "Dismiss me?", tab="w1:t2"),
    _q(11, "-1 hours", "w1:p2", "And me?", tab="w1:t2"),
    _q(12, "-1 hours", "w1:p9", "Pane without a tab?"),
    _q(13, "-1 hours", "w1:p4", "Pane herdr does not list?"),
    ("cli", {"argv": ["dismiss", "10"], "config": ON, "pane_tabs": TABS, "labels": {"w1:t2": "❓3 w1:t2-tab"}}),
    ("cli", {"argv": ["dismiss", "10"], "config": ON, "pane_tabs": TABS}),
    ("cli", {"argv": ["dismiss", "Q12"], "config": ON, "pane_tabs": TABS}),
    ("cli", {"argv": ["dismiss", "13"], "config": ON, "pane_tabs": TABS}),
    # the rename fails: the decoration stays recorded
    _q(14, "-1 hours", "w1:p2", "Rename fails?", tab="w1:t2"),
    ("cli", {"argv": ["dismiss", "11"], "config": 'tab_prefix = "!"\n', "pane_tabs": TABS,
             "labels": {"w1:t2": "w1:t2-tab"}, "rename_fails": True}),
    ("cli", {"argv": ["dismiss", "14"], "config": 'tab_prefix = "!"\n', "pane_tabs": TABS,
             "labels": {"w1:t2": "!1 ❓2 w1:t2-tab"}}),
    # unnamed tabs (their label is their position)
    _q(15, "-1 hours", "w1:p2", "Unnamed one?", tab="w1:t2"),
    _q(16, "-1 hours", "w1:p2", "Unnamed two?", tab="w1:t2"),
    ("cli", {"argv": ["dismiss", "15"], "config": ON, "pane_tabs": TABS, "labels": {"w1:t2": "6"}}),
    ("cli", {"argv": ["dismiss", "16"], "config": ON + "prefix_unnamed_tabs = true\n", "pane_tabs": TABS,
             "labels": {"w1:t2": "６"}}),
    _q(17, "-1 hours", "w1:p2", "Settings of other types?", tab="w1:t2"),
    _q(18, "-1 hours", "w1:p2", "Broken settings?", tab="w1:t2"),
    ("cli", {"argv": ["dismiss", "17"], "config": "tab_prefix = 7\nprefix_unnamed_tabs = 1\n", "pane_tabs": TABS,
             "labels": {"w1:t2": "²"}}),
    ("cli", {"argv": ["dismiss", "18"], "config": "tab_prefix = [\n", "pane_tabs": TABS,
             "labels": {"w1:t2": "❓1 x"}}),
    # herdr lists no panes: the token only
    _q(19, "-1 hours", "w1:p2", "herdr unreachable?", tab="w1:t2"),
    ("cli", {"argv": ["dismiss", "19"], "config": ON, "pane_tabs": None}),
    # the popup's submit of drafts (service.submit, no delivery)
    _q(20, "-1 hours", "w1:p1", "Pick some?"),
    _q(21, "-1 hours", "w1:p1", "Typed only?"),
    _q(22, "-1 hours", "w1:p2", "No draft?", tab="w1:t2"),
    _q(23, "-1 hours", "w1:p2", "Closed with a draft?", status="dismissed", tab="w1:t2"),
    _q(24, "-1 hours", "w1:p2", "Odd draft?", tab="w1:t2"),
    _draft(20, {"choices": ["Postgres", "a, b", 'say "hi"', "x·y"], "notes": {"Postgres": "keep it", "a, b": "",
                                                                           "absent": "dropped"}, "text": "more"}),
    _draft(21, {"choices": [], "notes": {}, "text": "just this"}),
    _draft(23, {"choices": ["late"], "notes": {}, "text": None}),
    _draft(24, '{"choices": ["ü\\u00fc"],"text":"  t  ", "extra": 1.50}'),
    ("submit", {"qids": [20, 21, 22, 23, 24, 99], "config": ON, "pane_tabs": TABS,
                "labels": {"w1:t1": "❓5 my-tab", "w1:t2": "❓3 w1:t2-tab"}}),
    # `inbox open`: the focus request and the popup, then the popup taking and resolving it
    ("cli", {"argv": ["open"]}),
    ("cli", {"argv": ["open", "--no-hold"], "open_error": "server_not_running"}),
    ("cli", {"argv": ["open", "-h"]}),
    ("cli", {"argv": ["open", "--question"]}),
    ("cli", {"argv": ["open", "--question", "1", "--pane", "w1:p1"]}),
    ("cli", {"argv": ["open", "--pane", "w1:p1", "--pane", "w1:p2"]}),
    ("cli", {"argv": ["open", "--question=QQ12"]}),
    ("cli", {"argv": ["open", "--g", "x"]}),
    ("cli", {"argv": ["open", "--session", "nope"]}),
    ("cli", {"argv": ["open", "--pane", "a b"]}),
    ("cli", {"argv": ["open", "--no-hold=1"]}),
    ("cli", {"argv": ["open", "--q=Q5", "extra"]}),
    ("cli", {"argv": ["open", "--question", ""]}),
    ("cli", {"argv": ["open", "--question", "Q20"]}),
    ("cli", {"argv": ["open", "--question", "qnap:Q12", "--no-hold"]}),
    ("cli", {"argv": ["open", "--group", "١٢"]}),
    ("cli", {"argv": ["open", "--session", S_NEW.upper()]}),
    ("cli", {"argv": ["open", "--pane", "w1:p2"], "popup_open": True}),
    ("cli", {"argv": ["open", "--pane", "w1:p2"], "open_error": "pane_not_found"}),
    _q(30, "-1 hours", "w1:p5", "Deep link one?", session=S_OLD, tab="w1:t5"),
    _q(31, "-1 hours", "w1:p5", "Deep link two?", session=S_OLD, tab="w1:t5"),
    ("sql", {"sql": "INSERT INTO question_ext (id, group_id) VALUES (31, 30)", "params": []}),
    ("sql", {"sql": "INSERT INTO remote_machines (machine_id, label, db_id) VALUES ('m1', 'qnap', 'd1'),"
                    " ('m2', 'other', 'd2')", "params": []}),
    ("sql", {"sql": "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json,"
                    " first_seen_at) VALUES ('m1', 'd1', 12, 'k12', 'open', ?, 'x'),"
                    " ('m1', 'd1', 13, 'k13', 'answered', ?, 'x'), ('m2', 'd2', 12, 'k2', 'open', ?, 'x')",
             "params": [json.dumps({"id": 12, "group": 12, "status": "open", "question": "Remote?"}),
                        json.dumps({"id": 13, "group": 13, "status": "answered", "question": "Done?"}),
                        json.dumps({"id": 12, "group": 7, "status": "open", "question": "Other?"})]}),
    ("deeplink", {"request": {"kind": "question", "value": "31"}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "question", "value": " Q30 "}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "question", "value": "29"}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "question", "value": "x"}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "group", "value": "30", "hold": False}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "group", "value": "31"}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "pane", "value": "w1:p5"}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "pane", "value": "w1:p0"}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "session", "value": S_OLD.upper()}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "session", "value": "nope"}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "question", "value": "qnap:12"}, "nonce": "n1",
                  "machines": [{"label": "qnap", "id": "m1"}]}),
    ("deeplink", {"request": {"kind": "question", "value": "m1:Q12"}, "nonce": "n1",
                  "machines": [{"label": "qnap", "id": "m1"}]}),
    ("deeplink", {"request": {"kind": "question", "value": "qnap:13"}, "nonce": "n1",
                  "machines": [{"label": "qnap", "id": "m1"}]}),
    ("deeplink", {"request": {"kind": "question", "value": "qnap:12"}, "nonce": "n1", "machines": []}),
    ("deeplink", {"request": {"kind": "question", "value": "other:12"}, "nonce": "n1",
                  "machines": [{"label": "other", "id": "m2"}, {"label": "qnap", "id": "m1"}]}),
    ("deeplink", {"request": {"kind": "group", "value": "30"}, "nonce": "n2"}),
    ("deeplink", {"request": {"kind": "group", "value": "30"}, "nonce": None}),
    ("deeplink", {"request": {"kind": "group", "value": "30"}, "nonce": "n1", "age": 6}),
    ("deeplink", {"request": {"kind": "group", "value": "30"}, "nonce": "n1", "age": -1}),
    ("deeplink", {"request": {"kind": "bogus", "value": "30"}, "nonce": "n1"}),
    ("deeplink", {"request": {"kind": "group", "value": 30, "hold": 0}, "nonce": "n1"}),
    ("deeplink", {"raw": "{not json", "nonce": "n1"}),
    ("deeplink", {"raw": "[1, 2]", "nonce": "n1"}),
    ("deeplink", {"nonce": "n1"}),
    # `inbox deliver` (the popup's detached sender): only answered, undelivered, unseen ones
    _q(40, "-1 hours", "w1:p6", "Deliver me?", status="answered", answer="yes", tab="w1:t6"),
    _q(41, "-1 hours", "w1:p6", "Delivered already?", status="answered", answer="no", tab="w1:t6"),
    _q(42, "-1 hours", "w1:p6", "Seen already?", status="answered", answer="ok", tab="w1:t6"),
    _q(43, "-1 hours", "w1:p6", "Still open?", tab="w1:t6"),
    ("sql", {"sql": "UPDATE questions SET delivered_at = '2001-01-01T00:00:00+00:00' WHERE id = 41", "params": []}),
    ("sql", {"sql": "UPDATE questions SET acknowledged_at = '2001-01-01T00:00:00+00:00' WHERE id = 42",
             "params": []}),
    ("cli", {"argv": ["deliver", "Q43", "41", "42", "99"]}),
    ("cli", {"argv": ["deliver", "40", "q41"], "delivery": {"error": "pending: busy"}}),
    ("cli", {"argv": ["deliver", "--force", "40"], "delivery": {"delivered": True}}),
    ("cli", {"argv": ["deliver", "40"]}),
    ("cli", {"argv": ["deliver"]}),
    ("cli", {"argv": ["deliver", "QQ1"]}),
    ("cli", {"argv": ["deliver", "-h"]}),
    # deliver_batch's preamble: the worker made sure of, answers of another herdr server left to it
    ("sql", {"sql": "UPDATE questions SET status = 'answered', answer = 'a', delivered_at = NULL,"
                    " delivery_error = NULL, acknowledged_at = NULL WHERE id IN (20, 21, 22, 24)", "params": []}),
    ("sql", {"sql": "INSERT INTO question_ext (id, group_id, herdr_server) VALUES (21, 21, '/srv/other'),"
                    " (22, 22, '/srv/mine'), (24, 24, '/srv/third') ON CONFLICT(id) DO UPDATE SET"
                    " herdr_server = excluded.herdr_server", "params": []}),
    # sync-tokens: every pane's token, then every tab's prefix (herdr lists the tabs)
    ("sync", {"argv": ["sync-tokens"], "config": ON, "pane_tabs": TABS + [["w1:p1", "w1:t1"]],
              "all_tabs": ["w1:t1", "w1:t2", "w1:t9", "w1:t0"],
              "labels": {"w1:t1": "Billing API", "w1:t2": "!7 ❓1 w1:t2-tab", "w1:t9": "❓3 stale"}}),
    ("sync", {"argv": ["sync-tokens"], "config": ON, "pane_tabs": TABS, "all_tabs": ["w1:t1"],
              "labels": {"w1:t1": "❓4 Billing API"}}),
    ("sync", {"argv": ["sync-tokens", "--remove-prefixes"], "config": ON, "pane_tabs": TABS,
              "all_tabs": ["w1:t1", "w1:t2"], "labels": {"w1:t1": "❓4 Billing API", "w1:t2": "6"}}),
    ("sync", {"argv": ["sync-tokens"], "config": ON, "pane_tabs": None}),
    ("sync", {"argv": ["sync-tokens"], "config": ON, "pane_tabs": TABS, "all_tabs": None}),
    ("cli", {"argv": ["sync-tokens", "-h"]}),
    # radar-sidebar over radar's own blocks (tests/fixtures/radar)
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font"}, "radar": False}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": None, "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font"}, "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font", "edited": True}, "radar": True}),
    ("radar", {"argv": ["radar-sidebar", "--remove"], "herdr_config": {"block": "dark-font", "edited": True},
               "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "light-text", "crlf": True}, "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-no-vendors"}, "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font",
               "replace": ["# >>> herdr-radar sidebar block", "# radar was here"]}, "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font", "replace": ["[keys]", "[keys"]},
               "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font", "replace":
               ["[ui.sidebar.spaces]\n", 'note = """multi [\nline ]"""\n[ui.sidebar.spaces]\n']}, "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font", "replace":
               ["[ui.sidebar.spaces]\n", '[[ui.sidebar.agents.extra]]\nname = "x"\n[ui.sidebar.spaces]\n']},
               "radar": True}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font"}, "radar": True,
               "radar_config": 'extra_tokens = ["$inbox_q"]\n'}),
    ("radar", {"argv": ["radar-sidebar", "--remove"], "herdr_config": {"block": "dark-font"}, "radar": True,
               "radar_config": 'extra_tokens = ["$inbox_q"]\n'}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font"}, "radar": True,
               "radar_config": "extra_tokens = []\n# $inbox_q in a comment\n[deep]\nextra_tokens = [{ token = \"$other\" }]\n"}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font"}, "radar": True,
               "radar_config": "[a.b]\nextra_tokens = [{ token = \"inbox_q\" }]\n"}),
    ("radar", {"argv": ["radar-sidebar"], "herdr_config": {"block": "dark-font"}, "radar": True,
               "radar_config": "broken = [\n"}),
    ("radar", {"argv": ["radar-sidebar", "-h"], "herdr_config": None, "radar": True}),
    ("radar_missing", {"herdr_config": {"block": "dark-font"}, "radar": True}),
    ("radar_missing", {"herdr_config": {"block": "dark-font", "edited": True}, "radar": True}),
    ("radar_missing", {"herdr_config": {"block": "dark-font"}, "radar": False}),
    ("radar_missing", {"herdr_config": {"block": "dark-font"}, "radar": True,
                       "config": ON + "radar_sidebar_hint = false\n"}),
    ("radar_missing", {"herdr_config": None, "radar": True, "config": ON}),
    ("preamble", {"qids": [20, 21, 22], "retry": False, "server": "/srv/mine"}),
    ("preamble", {"qids": [24, 21], "retry": False, "server": "/srv/other"}),
    ("preamble", {"qids": [21, 20], "retry": True, "server": "/srv/mine"}),
    ("preamble", {"qids": [22], "retry": False, "server": "/srv/other"}),
]


RADAR_BEFORE = '# my herdr config\n[keys]\nprefix = "ctrl-b"\n\n[ui]\nsidebar_width = 40   # keep "this" [as is]\n\n'
RADAR_AFTER = "\n# trailing comment, kept\n"


def radar_text(spec: dict) -> str:
    """herdr's config around one of radar's blocks (tests/test_radar.py's BEFORE/AFTER), with
    ``replace`` [old, new] once, CRLF line endings for ``crlf``, our cells already in for
    ``edited`` (radar.edited)."""
    from herdr_inbox import radar

    block = (Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "radar"
             / f"{spec['block']}.block.toml").read_text(encoding="utf-8")
    text = RADAR_BEFORE + block + RADAR_AFTER
    if spec.get("replace"):
        text = text.replace(spec["replace"][0], spec["replace"][1], 1)
    if spec.get("edited"):
        text = radar.edited(text, False)
    return text.replace("\n", "\r\n") if spec.get("crlf") else text


def _herdr_config(path: Path, args: dict, radar_dir: Path) -> None:
    """The step's herdr config.toml (or none) and radar's own config.toml (or none)."""
    path.unlink(missing_ok=True)
    if args.get("herdr_config") is not None:
        path.write_bytes(radar_text(args["herdr_config"]).encode())
    (radar_dir / "config.toml").unlink(missing_ok=True)
    if args.get("radar_config") is not None:
        (radar_dir / "config.toml").write_text(args["radar_config"])


def _deeplink(deeplink, path: Path, work: Path, args: dict) -> dict:
    """The popup's side of a deep link (tui.on_mount): the step's request file (``request``
    with nonce ``n1``, ``age`` seconds old; or ``raw`` text; or none), taken with the step's
    nonce (anything it raises is no request, as the popup catches it), then resolved."""
    import time

    from herdr_inbox import store as st

    target = deeplink.request_path()
    target.unlink(missing_ok=True)
    if "raw" in args:
        target.write_text(args["raw"])
    elif "request" in args:
        data = {**args["request"], "created_at": time.time() - args.get("age", 0), "nonce": "n1"}
        target.write_text(json.dumps(data))
    os.environ.pop(deeplink.NONCE_ENV, None)
    if args["nonce"]:
        os.environ[deeplink.NONCE_ENV] = args["nonce"]
    try:
        request = deeplink.consume_request()
    except Exception:  # noqa: BLE001 - the popup's own guard: a deep link never breaks it
        request = None
    finally:
        os.environ.pop(deeplink.NONCE_ENV, None)
    out = {"request": request, "left": target.exists()}
    target.unlink(missing_ok=True)
    if request:
        with st.Store(path) as s:
            got = deeplink.resolve(s, request)
        out["resolved"] = None if got is None else [list(got[0]), got[1]]
        out["describe"] = deeplink.describe(request)
    return out


def _tables(path: Path) -> dict:
    from capture_goldens import _store_tables

    return _store_tables(path, ("questions", "question_ext", "tab_decor"))


class _Stderr(logging.Handler):
    """Python's CLI logging (``inbox: <message>``) to whatever sys.stderr is now."""

    def emit(self, record: logging.LogRecord) -> None:
        sys.stderr.write(f"inbox: {record.getMessage()}\n")


def capture_cli(out: Path, work: Path) -> Path:
    from herdr_inbox import deeplink, herdr, radar, remote, service, store as st, tabs, worker
    from herdr_inbox import cli

    path = work / "cli.db"
    radar_dir = work / "radar-config"
    radar_dir.mkdir(parents=True, exist_ok=True)
    herdr_cfg = Path(os.environ["HERDR_INBOX_HERDR_CONFIG"])
    herdr_cfg.parent.mkdir(parents=True, exist_ok=True)
    os.environ["HERDR_INBOX_DB"] = str(path)
    cfg = Path(os.environ["HERDR_PLUGIN_CONFIG_DIR"]) / "config.toml"
    projects = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "projects" / "-r"
    projects.mkdir(parents=True, exist_ok=True)
    effects: list = []
    world: dict = {}

    def deliver_batch(store, qs, retry=False, force=False, verify_later=None):
        effects.append(["deliver", [q.id for q in qs], retry, force])
        outcome = world.get("delivery") or {}
        for q in qs:
            if outcome.get("delivered"):
                store.db.execute("UPDATE questions SET delivered_at = ?, delivery_error = NULL WHERE id = ?",
                                 (st.now(), q.id))
            else:
                store.db.execute("UPDATE questions SET delivery_error = ? WHERE id = ?", (outcome.get("error"), q.id))

    def rename_tab(tab, label):
        effects.append(["rename_tab", tab, label])
        if world.get("rename_fails"):
            return "refused"
        world.setdefault("labels", {})[tab] = label
        return None

    def open_popup(env=None):
        env = {k: ("<nonce>" if k == deeplink.NONCE_ENV else v) for k, v in (env or {}).items()}
        effects.append(["open_popup", env])
        return world.get("open_error")

    def herdr_call(*args, **kw):
        if args[:2] == ("plugin", "list"):
            effects.append(["radar_installed"])
            plugins = [{"plugin_id": "leavemyyard.herdr-inbox"}]
            if world.get("radar"):
                plugins.append({"plugin_id": radar.RADAR_ID})
            return {"plugins": plugins}, None
        raise AssertionError(f"unstubbed herdr call {args}")

    def plugin_config_dir(plugin_id):
        assert plugin_id == radar.RADAR_ID, plugin_id
        effects.append(["radar_config_dir"])
        return str(radar_dir) if world.get("radar_config") is not None else None

    def run_raw(*args, **kw):
        assert args == ("server", "reload-config"), args
        effects.append(["reload_config"])
        return "", None

    stubs = {
        (herdr, "set_open_count"): lambda pane, n: effects.append(["token", pane, n]),
        (herdr, "pane_tabs"): lambda: effects.append(["pane_tabs"]) or (
            None if world.get("pane_tabs") is None else {p: t for p, t in world["pane_tabs"]}),
        (herdr, "tab_label"): lambda tab: effects.append(["tab_label", tab]) or world.get("labels", {}).get(tab),
        (herdr, "rename_tab"): rename_tab,
        (herdr, "notify"): lambda title, body: effects.append(["notify", title, body]),
        (herdr, "server_id"): lambda: world.get("server"),
        (service, "deliver_batch"): deliver_batch,
        (service, "_ensure_worker"): lambda server=None: effects.append(["ensure", server]),
        (worker, "ensure"): lambda server=None: effects.append(["ensure", server]),
        (herdr, "open_popup"): open_popup,
        (herdr, "all_tabs"): lambda: effects.append(["all_tabs"]) or (
            None if world.get("all_tabs") is None
            else [{"tab_id": t, "label": world.get("labels", {}).get(t)} for t in world["all_tabs"]]),
        (herdr, "call"): herdr_call,
        (herdr, "plugin_config_dir"): plugin_config_dir,
        (herdr, "run_raw"): run_raw,
        (remote, "enabled_machines"): lambda ttl=60.0: effects.append(["machines"]) or world.get("machines", []),
    }
    saved = {k: getattr(*k) for k in stubs}
    real_deliver = service.deliver_batch
    root = logging.getLogger()
    handler = _Stderr()
    root.addHandler(handler)  # basicConfig in cli.main is then a no-op
    root.setLevel(logging.WARNING)
    steps = []
    last = None  # the tables after the previous step: a step that leaves them is "unchanged"
    try:
        for (mod, name), fn in stubs.items():
            setattr(mod, name, fn)
        for op, args in CLI_OPS:
            effects.clear()
            world.clear()
            world.update(copy.deepcopy(args))  # a rename changes the world's labels, never the step
            if "config" in args:
                cfg.write_text(args["config"])
            for sid, records in args.get("files", {}).items():
                (projects / f"{sid}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in records))
            for key in ("CLAUDE_CODE_SESSION_ID", "HERDR_PANE_ID"):
                os.environ.pop(key, None)
            me = args.get("me") or {}
            if me.get("session"):
                os.environ["CLAUDE_CODE_SESSION_ID"] = me["session"]
            if me.get("pane"):
                os.environ["HERDR_PANE_ID"] = me["pane"]
            step = {"op": op, "args": args}
            if op == "migrate":
                st.Store(path, migrate=True).close()
            elif op == "sql":
                with st.Store(path) as s:
                    s.db.execute(args["sql"], args["params"])
            elif op == "submit":
                with st.Store(path) as s:
                    step["result"] = [[q.id, q.answer] for q in service.submit(s, args["qids"], deliver=False)]
            elif op == "radar_missing":
                _herdr_config(herdr_cfg, args, radar_dir)
                step["result"] = radar.missing()
            elif op == "deeplink":
                step["result"] = _deeplink(deeplink, path, work, args)
            elif op == "preamble":
                kept: list = []
                service.deliver_batch = real_deliver
                with st.Store(path) as s:
                    real_claim = s.claim_delivery
                    s.claim_delivery = lambda qid: kept.append(qid) and None
                    try:
                        real_deliver(s, [s.get(i) for i in args["qids"]], retry=args["retry"])
                    finally:
                        s.claim_delivery = real_claim
                service.deliver_batch = deliver_batch
                step["result"] = kept
            else:
                if op == "radar":
                    _herdr_config(herdr_cfg, args, radar_dir)
                if args.get("sessions"):
                    with st.Store(path) as s:
                        for sid in args["sessions"]:
                            s.record_session(sid, str(projects / f"{sid}.jsonl"))
                stdout, stderr = io.StringIO(), io.StringIO()
                held = deeplink.hold_popup_lock() if args.get("popup_open") else None
                request = deeplink.request_path()
                request.unlink(missing_ok=True)
                with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                    try:
                        code = cli.main(list(args["argv"]))
                    except SystemExit as e:
                        code = e.code
                if held is not None:
                    held.close()
                if request.exists():  # what `inbox open` left for the popup
                    data = json.loads(request.read_text())
                    step["request"] = {k: ("<" + k + ">" if k in ("nonce", "created_at") else v)
                                       for k, v in data.items()}
                    request.unlink()
                err = re.sub(r"(config\.toml: ).*", r"\1<reason>", stderr.getvalue().replace(str(work), "<work>"))
                step.update(code=code, stdout=stdout.getvalue().replace(str(work), "<work>"), stderr=err)
                if op == "radar":
                    step["herdr_config"] = herdr_cfg.read_bytes().decode() if herdr_cfg.exists() else None
                    bak = herdr_cfg.with_name("config.toml.inbox-bak")
                    step["backup"] = bak.exists()
                    bak.unlink(missing_ok=True)
            step["effects"] = list(effects)
            migrated = op == "migrate" or any(s["op"] == "migrate" for s in steps)
            dump = _tables(path) if migrated else None
            step["tables"] = "unchanged" if migrated and dump == last else dump
            last = dump
            steps.append(step)
            if not migrated:  # Python's Store() created an empty file: the next step starts without one
                for f in work.glob("cli.db*"):
                    f.unlink()
    finally:
        for (mod, name), fn in saved.items():
            setattr(mod, name, fn)
        root.removeHandler(handler)
        for key in ("CLAUDE_CODE_SESSION_ID", "HERDR_PANE_ID"):
            os.environ.pop(key, None)
    del tabs
    doc = {"source": "cli.main over CLI_OPS (tools/capture_goldens.py --only cli)", "steps": steps}
    target = out / "cli.json"
    target.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n")
    return target

