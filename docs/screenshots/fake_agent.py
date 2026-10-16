#!/usr/bin/env python3
"""A stand-in coding agent for the README captures: prints a scripted transcript.

It makes no API calls and runs no model. It only draws a short, scripted agent session
for one of the demo tabs, tells herdr that an agent named ``claude`` lives in this pane
(``herdr pane report-agent``), and echoes whatever the inbox types into it, so the
answering flow can be recorded end to end.

Usage: fake_agent.py <billing|search|docs|release>
"""

import os
import re
import subprocess
import sys
import termios
import time

DIM, BOLD, RESET = "\x1b[2m", "\x1b[1m", "\x1b[0m"
ORANGE, GREEN, GREY, BLUE = "\x1b[38;2;215;119;87m", "\x1b[38;2;78;186;101m", "\x1b[38;2;153;153;153m", "\x1b[38;2;177;185;249m"
DOT = f"{BOLD}●{RESET}"

SCRIPTS = {
    "billing": ("~/acme/billing-api", "Add an invoice store and warm the caches on deploy", [
        ("tool", "Read(src/invoices/models.py)", "Read 142 lines"),
        ("tool", "Grep(\"cache\", path: \"deploy/\")", "Found 6 files"),
        ("ask", "Invoice storage and caching", "Q2, Q3"),
        ("say", "While Q2 and Q3 are open I'll write the parts that don't depend on them."),
        ("tool", "Write(src/invoices/exporter.py)", "Wrote 88 lines"),
        ("tool", "Bash(uv run pytest tests/invoices -q)", "14 passed in 1.92s"),
    ], "Writing the deploy hook…", "SQLite for the invoice store, redis and CDN warmed on deploy. Updating the exporter."),
    "search": ("~/acme/search", "Review PR #42 and get it merged", [
        ("tool", "Bash(gh pr view 42 --json title,commits)", "\"Search: split the query parser\", 7 commits"),
        ("tool", "Bash(gh pr checks 42)", "All checks have passed"),
        ("ask", "Merge mode", "Q1"),
        ("say", "Moving on to the follow-up issues while you decide."),
        ("tool", "Bash(gh issue list --label search)", "4 open issues"),
    ], "Triaging follow-ups…", "Merging PR #42 as decided."),
    "docs": ("~/acme/docs-site", "Prepare the v2 docs for publishing", [
        ("tool", "Bash(pnpm build)", "Built 214 pages in 38.4s"),
        ("tool", "Bash(pnpm linkcheck)", "0 broken links"),
        ("ask", "Rollout", "Q4"),
        ("say", "The build is green; drafting the changelog page meanwhile."),
    ], "Drafting the changelog…", "Scheduling the docs publish as decided."),
    "onboarding": ("~/acme/web-app", "Rework the onboarding flow's empty states", [
        ("tool", "Read(src/onboarding/Steps.tsx)", "Read 211 lines"),
        ("tool", "Edit(src/onboarding/EmptyState.tsx)", "Updated with 34 additions"),
        ("tool", "Bash(pnpm test onboarding)", "22 passed"),
    ], "Updating snapshots…", "Noted."),
    "perf": ("~/acme/search", "Profile the slow /search endpoint", [
        ("tool", "Bash(py-spy record -o profile.svg -- python bench.py)", "Wrote profile.svg"),
        ("say", "Most time goes to tokenising the query twice; fixing that first."),
        ("tool", "Edit(search/query.py)", "Updated with 9 additions and 14 removals"),
    ], "Re-running the benchmark…", "Noted."),
    "release": ("~/acme/release", "Draft the 3.2 release notes", [
        ("tool", "Bash(git log v3.1..HEAD --oneline)", "61 commits"),
        ("say", "Grouping the commits by area."),
        ("tool", "Write(notes/3.2.md)", "Wrote 47 lines"),
    ], "Grouping commits…", "Noted."),
}


def width() -> int:
    try:
        return os.get_terminal_size().columns
    except OSError:
        return 80


def report(state: str) -> None:
    pane = os.environ.get("HERDR_PANE_ID")
    if pane:
        subprocess.run(["herdr", "pane", "report-agent", pane, "--source", "readme-demo", "--agent", "claude",
                        "--state", state], capture_output=True)


def boxed(rows: list[str], w: int, color: str) -> None:
    """A rounded box; ``rows`` may hold colour codes (padding counts visible text only)."""
    print(f"{color}╭{'─' * (w - 2)}╮{RESET}")
    for row in rows:
        visible = len(re.sub(r"\x1b\[[0-9;]*m", "", row))
        print(f"{color}│{RESET}{row}{' ' * (w - 2 - visible)}{color}│{RESET}")
    print(f"{color}╰{'─' * (w - 2)}╯{RESET}")


def banner(cwd: str) -> None:
    boxed([f" {ORANGE}✻{RESET} Welcome to {BOLD}Claude Code{RESET}", "", f"   {GREY}cwd: {cwd}{RESET}"],
          min(width() - 2, 52), ORANGE)
    print()


def step(kind: str, *args: str) -> None:
    if kind == "tool":
        name, result = args
        tool, _, rest = name.partition("(")
        print(f"{GREEN}●{RESET} {BOLD}{tool}{RESET}({rest}")
        print(f"  {GREY}⎿  {result}{RESET}\n")
    elif kind == "ask":
        title, refs = args
        print(f"{GREEN}●{RESET} {BOLD}herdr-inbox - ask_operator{RESET} (MCP)(title: \"{title}\")")
        print(f"  {GREY}⎿  Filed {refs}. Answers arrive later as [inbox] messages; keep working.{RESET}\n")
    else:
        print(f"{DOT} {args[0]}\n")


def prompt_box() -> None:
    boxed([" > "], min(width() - 2, 100), GREY)


def main() -> None:
    cwd, task, steps, busy, reply = SCRIPTS[sys.argv[1]]
    sys.stdout.write("\x1b[2J\x1b[H")
    banner(cwd)
    print(f"{GREY}>{RESET} {task}\n")
    for s in steps:
        step(*s)
    idle = os.environ.get("DEMO_STATE") == "idle"  # done with everything that does not need the answers
    if idle:
        print(f"{DOT} Everything that doesn't depend on the open questions is done.\n")
    else:
        print(f"{ORANGE}✻{RESET} {ORANGE}{busy}{RESET} {GREY}(esc to interrupt){RESET}\n")
    prompt_box()
    sys.stdout.flush()
    report("idle" if idle else "working")
    # read what the inbox types (bracketed paste + Enter), without the tty echoing it
    fd = sys.stdin.fileno()
    attrs = termios.tcgetattr(fd)
    attrs[3] &= ~termios.ECHO
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    for line in sys.stdin:
        text = re.sub(r"\x1b\[20[01]~", "", line).strip()
        if not text:
            continue
        sys.stdout.write("\x1b[3A\x1b[J")  # replace the prompt box
        print(f"{GREY}>{RESET} {BLUE}{text}{RESET}\n")
        print(f"{DOT} {reply}\n")
        prompt_box()
        sys.stdout.flush()
    time.sleep(3600)


if __name__ == "__main__":
    main()
