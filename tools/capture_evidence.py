"""Capture tests/fixtures/evidence.json: how Python's `transcript.own_answer_qids` splits and
reads shell commands (its `shlex` setup included) and how `own_answer_succeeded` reads CLI
output, over the commands below. `src/claude/evidence/tests` replays it on the Rust port.

    python3 -I tools/capture_evidence.py [--out FILE]

Standard library only; nothing outside this checkout is read.
"""

from __future__ import annotations

import argparse
import json
import shlex
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from herdr_inbox import transcript  # noqa: E402

COMMANDS = [
    # tests/test_failed_pile.py (review #44 r3 and the final pass)
    'grep -n "answer 7" ~/Code/herdr-inbox/worker.log',
    'gh pr comment 44 -R LeaveMyYard/herdr-inbox --body "run inbox answer 7 x to reproduce"',
    "inbox answer 7 x; grep 'Q7' ~/.local/state/herdr-inbox/worker.log",
    'B=~/.claude/plugins/data/herdr-inbox-herdr-inbox/venv-0.10.4/bin/inbox\n$B answer 7 "x"',
    'cd ~/Code/herdr-inbox && uv run inbox answer Q7 "x"',
    "/home/u/.venv/bin/inbox answer 7 'x'",
    "python3 -m herdr_inbox.cli answer 7 x",
    ("B=~/.claude/plugins/data/herdr-inbox-herdr-inbox/venv-0.10.4/bin/inbox\n"
     '$B answer 159 "Keep as built (merged in #1943): placeholder; goes away with v1" &&\n'
     '$B answer 160 "Keep as built: prev/next + counter" &&\n'
     '$B answer 165 "(a) as built (allowance / week); log it" &&\n'
     '$B answer 166 "x | y"'),
    'HERDR_INBOX_DB=~/copy.db inbox answer 7 "x"',
    'XDG_STATE_HOME=/tmp/s ~/.venv/bin/inbox answer 7 "x"',
    'export HERDR_INBOX_DB=~/copy.db\ninbox answer 7 "x"',
    'HERDR_INBOX_DB=~/copy.db && inbox answer 7 "x"',
    # the splitter's corners: quotes, escapes, comments, punctuation runs, assignments
    "inbox answer 7 'unclosed",
    'inbox answer 7 "trailing escape \\',
    "inbox answer 7 x\\",
    "inbox answer 7 # a comment\ninbox answer 8 y",
    "inbox answer 7 x#not-a-comment\ninbox answer 8 y",
    "echo hi # inbox answer 7 x",
    "inbox answer 7 x;inbox answer 8 y&&inbox answer 9 z||inbox answer 10 w|cat",
    "inbox answer 7 x ;; inbox answer 8 y",
    "inbox answer '' 7",
    "inbox answer \"\" 7",
    "inbox\\ answer 7 x",
    "in\\box answer 7 x",
    "inbox an'swer' 7 x",
    'inbox answer "7" x',
    "inbox answer Q7 x",
    "inbox answer q7 x",
    "inbox answer 0007 x",
    "inbox answer 7x x",
    "inbox answer -7 x",
    "inbox answer +7 x",
    "inbox answer ٧ x",
    "inbox answer 1_000 x",
    "inbox answer 99999999999999999999999 x",
    "inbox answer",
    "inbox answer 7",
    "inbox list; inbox answer 7 x",
    "./inbox answer 7 x",
    "xinbox answer 7 x",
    "inbox/ answer 7 x",
    "A=1 B=2 inbox answer 7 x",
    "A=1 B=2",
    "A= inbox answer 7 x",
    "1A=x inbox answer 7 x",
    "_A=x inbox answer 7 x",
    "Ä=x inbox answer 7 x",
    "AÄ=inbox\n$AÄ answer 7 x",
    "I=inbox\n$I answer 7 x",
    "I=inbox\n${I} answer 7 x",
    "I=/opt/bin/inbox; $I answer 7 x",
    "I=inboxes\n$I answer 7 x",
    "$I answer 7 x",
    "I=inbox J=2\n$I answer 7 x",
    "uv run inbox answer 7 x",
    "uv run --project /r inbox answer 7 x",
    "uv run --project=/r inbox answer 7 x",
    "uv run -q --frozen --no-dev --directory /r inbox answer 7 x",
    "uv run --with x inbox answer 7 x",
    "uv run -p 3.12 inbox answer 7 x",
    "uv run --project",
    "uv run",
    "uv run python -m herdr_inbox.cli answer 7 x",
    "python -m herdr_inbox.cli answer 7 x",
    "/usr/bin/python3 -m herdr_inbox.cli answer 7 x",
    "python3.12 -m herdr_inbox.cli answer 7 x",
    "python3 -m herdr_inbox answer 7 x",
    "python3 -m",
    "cd /r &&\ninbox answer 7 x",
    "cd /r &&\n\ninbox answer 7 x",
    "true &\ninbox answer 7 x",
    "inbox answer 7 \"a ; b && c\" ; inbox answer 8 'd | e'",
    "inbox answer 7 \"say \\\"hi\\\" \\$x \\a\"",
    "inbox answer 7 'it\\'s' 8",
    "inbox\tanswer\t7\tx",
    "inbox answer 7 x\r\ninbox answer 8 y",
    "inbox answer\u00a07 x",
    "inbox answer 7 x\n",
    "X=inbox\n",
    "HERDR_INBOX_DB=a\ninbox answer 7 x",
    "inbox answer 7 x\nHERDR_INBOX_DB=a\ninbox answer 8 y",
    "echo HERDR_INBOX_DB=a; inbox answer 7 x",
    "env HERDR_INBOX_DB=a inbox answer 7 x",
    "HERDR_INBOX_DBX=a inbox answer 7 x",
    "'' inbox answer 7 x",
    "\"\" ; inbox answer 7 x",
    "inbox answer 7 x &",
    "inbox answer 7 x | tee log",
    "(inbox answer 7 x)",
    "{ inbox answer 7 x; }",
    "inbox answer 7 x >/dev/null 2>&1",
    "sudo inbox answer 7 x",
    "time inbox answer 7 x",
]

OUTPUTS = [
    ("Q7 answered, delivered to w1:p1", 7),
    ("Q7 answered, not delivered: no live agent in pane w1:p4\n", 7),
    ("inbox: herdr failed: x\nQ7 answered, pending: busy", 7),
    ("Q7 answered by its own conversation (seen there)", 7),
    ("  Q7 answered, delivered\n", 7),
    ("Q7 is not open\nQ7 answered, delivered to w1:p1", 7),
    ("Q70 answered, delivered", 7),
    ("Q7 answered", 7),
    ("1234: Q7 answered, delivered", 7),
    ("x\rQ7 answered, delivered", 7),
    ("x\u2028Q7 answered, delivered", 7),
    ("x\x1cQ7 answered, delivered", 7),
    ("x\x85Q7 answered, delivered", 7),
    ("Q7 answered,delivered", 7),
    ("", 7),
]


def tokens(command: str):
    try:
        lexer = shlex.shlex(command, posix=True, punctuation_chars=";&|\n")
        lexer.whitespace = " \t\r"
        lexer.whitespace_split = True
        return list(lexer)
    except ValueError as exc:
        return {"error": str(exc)}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "tests" / "fixtures" / "evidence.json"))
    args = ap.parse_args()
    data = {
        "source": "tools/capture_evidence.py (transcript.own_answer_qids, own_answer_succeeded)",
        "commands": [{"command": c, "tokens": tokens(c), "qids": sorted(transcript.own_answer_qids(c))}
                     for c in COMMANDS],
        "outputs": [{"output": o, "qid": q, "succeeded": transcript.own_answer_succeeded(o, q)}
                    for o, q in OUTPUTS],
    }
    Path(args.out).write_text(json.dumps(data, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
