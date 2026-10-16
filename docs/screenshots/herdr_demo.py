"""Capture the README screenshots from a real, isolated herdr with the inbox plugin linked.

    uv run --with pyte --with pillow python docs/screenshots/herdr_demo.py

Needs herdr >= 0.9, ``rsvg-convert`` and ``ffmpeg`` (for the GIF). Writes PNGs and
``answering.gif`` into docs/screenshots/.

Isolation: herdr runs with HOME and every XDG dir under ``.scratch/herdr-demo/`` (git
ignored, wiped at each run), its own API socket under ``$XDG_RUNTIME_DIR/herdr-inbox-demo``
(unix socket paths are length-limited) and its own inbox database. Nothing touches your
herdr, your config or your inbox. The server is stopped at the end, also on failure.

The tabs run ``fake_agent.py``: a scripted stand-in that prints a short agent transcript
and makes no API calls. Questions are filed through the real ``service.ask`` code path,
so the tab markers and the popup are the plugin's own.
"""

from __future__ import annotations

import json
import os
import shutil
import sqlite3
import subprocess
import sys
import time
from datetime import datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
from frame import frame_file  # noqa: E402
from term import Terminal  # noqa: E402

DEMO = ROOT / ".scratch" / "herdr-demo"
HOME = DEMO / "home"
RUN = Path(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid())) / "herdr-inbox-demo"
OUT = HERE / "herdr"
COLS, ROWS = 170, 46
PNG_WIDTH = 1840
PREFIX = "\x02"  # ctrl+b, herdr's default prefix

CONFIG = """\
onboarding = false

[update]
version_check = false
manifest_check = false

[terminal]
default_shell = "{shell}"

[[keys.command]]
key = "prefix+i"
type = "plugin_action"
command = "leavemyyard.herdr-inbox.open"
description = "question inbox"
"""

SHELL_RC = """\
PS1='\\[\\e[38;2;137;180;250m\\]\\W\\[\\e[0m\\] $ '
cd "$DEMO_CWD" 2>/dev/null
"""

TABS = [  # label, project dir, fake agent script
    ("Billing API", "billing-api", "billing"),
    ("Search refactor", "search", "search"),
    ("Docs site", "docs-site", "docs"),
    ("Onboarding", "web-app", "onboarding"),
    ("Perf audit", "perf", "perf"),
    ("Release notes", "release", "release"),
]

DB = [{"label": "Postgres", "description": "keeps the current ORM and migrations"},
      {"label": "SQLite", "description": "single file, no server"}, {"label": "DuckDB"}]


MERGE = [
    {"label": "merge commit", "description": "keeps the 7 commits",
     "preview": "*   Merge pull request #42\n|\\\n| * Split the query parser\n| * Parser tests\n| * ...\n|/\n* main"},
    {"label": "squash", "description": "one commit on main",
     "preview": "* Search: split the query parser (#42)\n* main"},
]


def isolated_env() -> dict[str, str]:
    env = {
        "PATH": f"{DEMO / 'bin'}:{os.environ['PATH']}",
        "HOME": str(HOME),
        "XDG_CONFIG_HOME": str(HOME / ".config"),
        "XDG_STATE_HOME": str(HOME / ".local/state"),
        "XDG_DATA_HOME": str(HOME / ".local/share"),
        "XDG_CACHE_HOME": str(HOME / ".cache"),
        "XDG_RUNTIME_DIR": str(DEMO / "run"),
        "HERDR_SOCKET_PATH": str(RUN / "s.sock"),
        "HERDR_INBOX_DB": str(DEMO / "inbox.db"),
        # the plugin's popup runs `uv run --project <plugin>`: keep its venv out of the checkout
        "UV_PROJECT_ENVIRONMENT": str(DEMO / "venv"),
        "UV_CACHE_DIR": os.environ.get("UV_CACHE_DIR") or str(Path.home() / ".cache" / "uv"),
        "TERM": "xterm-256color",
        "COLORTERM": "truecolor",
        "LANG": "C.UTF-8",
        "SHELL": "/bin/bash",
        "GIT_CONFIG_GLOBAL": str(HOME / ".gitconfig"),
    }
    return env


def prepare() -> dict[str, str]:
    shutil.rmtree(DEMO, ignore_errors=True)
    OUT.mkdir(exist_ok=True)
    for d in ("home/.config/herdr", "home/.local/state", "home/.local/share", "home/.cache", "run", "bin", "frames", "raw"):
        (DEMO / d).mkdir(parents=True, exist_ok=True)
    os.chmod(DEMO / "run", 0o700)
    RUN.mkdir(mode=0o700, parents=True, exist_ok=True)
    (HOME / ".gitconfig").write_text("[user]\n\tname = demo\n\temail = demo@example.com\n[init]\n\tdefaultBranch = main\n")
    shell = DEMO / "bin" / "demo-shell"
    (DEMO / "bashrc").write_text(SHELL_RC)
    shell.write_text(f"#!/bin/bash\nexec bash --noprofile --rcfile {DEMO / 'bashrc'} -i\n")
    shell.chmod(0o755)
    # the stand-in agent, installed as `claude` so herdr's process detection sees an agent
    agent = DEMO / "bin" / "claude"
    agent.write_text("#!/usr/bin/python3\n" + (HERE / "fake_agent.py").read_text().split("\n", 1)[1])
    agent.chmod(0o755)
    (HOME / ".config/herdr/config.toml").write_text(CONFIG.format(shell=shell))
    env = isolated_env()
    acme = HOME / "acme"
    for _, project, _ in TABS:
        (acme / project).mkdir(parents=True)
    git = ["git", "-C", str(acme)]
    subprocess.run([*git, "init", "-q"], env=env, check=True)
    subprocess.run([*git, "commit", "-q", "--allow-empty", "-m", "init"], env=env, check=True)
    return env


class Herdr:
    def __init__(self, env: dict[str, str]):
        self.env = env

    def raw(self, *args: str, check: bool = True) -> str:
        p = subprocess.run(["herdr", *args], env=self.env, capture_output=True, text=True, timeout=30)
        if check and p.returncode:
            raise RuntimeError(f"herdr {' '.join(args)}: {p.stdout}{p.stderr}")
        return p.stdout

    def __call__(self, *args: str) -> dict:
        out = self.raw(*args)
        return (json.loads(out) if out.strip() else {}).get("result") or {}


def wait_socket(h: Herdr, timeout: float = 15) -> None:
    end = time.time() + timeout
    while time.time() < end:
        if h.raw("pane", "list", check=False).find('"result"') >= 0:
            return
        time.sleep(0.2)
    raise TimeoutError("isolated herdr server did not come up")


def build_scene(h: Herdr) -> list[dict]:
    """Four tabs, each a fake agent in its project dir. Returns [{tab, pane, label}]."""
    ws = h("workspace", "list")["workspaces"][0]["workspace_id"]
    first_tab = h("tab", "list", "--workspace", ws)["tabs"][0]["tab_id"]
    first_pane = next(p["pane_id"] for p in h("pane", "list")["panes"] if p.get("tab_id") == first_tab)
    scene = []
    for i, (label, project, script) in enumerate(TABS):
        cwd = str(HOME / "acme" / project)
        if i == 0:
            tab, pane = first_tab, first_pane
            h("tab", "rename", tab, label)
        else:
            res = h("tab", "create", "--workspace", ws, "--cwd", cwd, "--label", label)
            tab, pane = res["tab"]["tab_id"], res["root_pane"]["pane_id"]
        # the Billing API agent has finished what it can and is idle: inbox 0.10 types an answer
        # into an idle agent at once (a working one gets it through its hooks, or after a grace)
        state = "DEMO_STATE=idle " if i == 0 else ""
        h("pane", "run", pane, f"cd {cwd} && clear && {state}exec claude {script}")
        scene.append({"tab": tab, "pane": pane, "label": label, "cwd": cwd})
    # a second pane in Docs site: a dev server, so the tab is not just one agent
    docs = scene[2]
    res = h("pane", "split", docs["pane"], "--direction", "right", "--cwd", docs["cwd"], "--no-focus")
    dev = res["pane"]["pane_id"]
    h("pane", "run", dev, "clear && printf '\\033[1;32m  VITE v5.4.2\\033[0m  ready in 412 ms\\n\\n"
      "  ➜  Local:   http://localhost:5173/\\n  ➜  Network: use --host to expose\\n\\n"
      "\\033[2m12:04:31\\033[0m \\033[36m[vite]\\033[0m page reload docs/v2/index.md\\n'")
    h.raw("workspace", "rename", ws, "acme", check=False)
    h("tab", "focus", scene[0]["tab"])
    return scene


def seed(env: dict[str, str], scene: list[dict]) -> None:
    """File the demo questions through the plugin's own code, as the agents' MCP servers would."""
    os.environ.update(env)
    for key in [k for k in os.environ if k.startswith("HERDR_") and k not in env]:
        del os.environ[key]
    from herdr_inbox import service
    from herdr_inbox.service import Caller
    from herdr_inbox.store import Store

    def caller(i: int) -> Caller:
        s = scene[i]
        return Caller(pane_id=s["pane"], tab_id=s["tab"], tab_label=s["label"], session_id=None, cwd=s["cwd"])

    store = Store()
    service.ask(store, caller(1), "Merge PR #42 with a merge commit or squash?", options=MERGE,
                header="Merge mode", asked_by="reviewer", context="7 commits, all checks green")                                                   # Q1
    service.ask_group(store, caller(0), [
        {"question": "Which database should the invoice store use?", "header": "DB choice", "options": DB},
        {"question": "Which caches should the API warm on deploy?", "header": "Cache", "multi_select": True,
         "options": ["redis", "in-process", "CDN"]}], context="Invoice rows ~1M/day", asked_by="planner",
        title="Invoice storage and caching")                                                               # Q2, Q3
    service.ask(store, caller(2), "Publish the v2 docs now or after the release?", options=["now", "after"],
                header="Rollout")                                                                           # Q4
    store.close()
    # the questions were asked a few minutes ago, not this second
    with sqlite3.connect(env["HERDR_INBOX_DB"]) as db:
        for qid, minutes in ((1, 9), (2, 4), (3, 4), (4, 2)):
            (created,) = db.execute("SELECT created_at FROM questions WHERE id = ?", (qid,)).fetchone()
            earlier = datetime.fromisoformat(created) - timedelta(minutes=minutes)
            db.execute("UPDATE questions SET created_at = ? WHERE id = ?", (earlier.isoformat(), qid))


class Shots:
    def __init__(self, term: Terminal):
        self.term = term
        self.frames: list[tuple[Path, float]] = []

    def png(self, name: str) -> None:
        raw = DEMO / "raw" / f"{name}.png"
        self.term.png(raw, "herdr", PNG_WIDTH)
        frame_file(raw, OUT / f"{name}.png")
        print("wrote", name)

    def frame(self, hold: float) -> None:
        raw = DEMO / "frames" / f"raw{len(self.frames):03d}.png"
        path = DEMO / "frames" / f"f{len(self.frames):03d}.png"
        self.term.png(raw, "herdr", 1200)
        frame_file(raw, path, pad=52)
        self.frames.append((path, hold))

    def gif(self, name: str) -> None:
        lst = DEMO / "frames" / "list.txt"
        lines = []
        for path, hold in self.frames:
            lines += [f"file '{path}'", f"duration {hold}"]
        lines.append(f"file '{self.frames[-1][0]}'")
        lst.write_text("\n".join(lines) + "\n")
        palette = DEMO / "frames" / "palette.png"
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", str(lst),
                        "-vf", "scale=920:-1:flags=lanczos,palettegen=stats_mode=diff", str(palette)], check=True)
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", str(lst),
                        "-i", str(palette), "-lavfi",
                        "scale=920:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle",
                        "-loop", "0", str(OUT / f"{name}.gif")], check=True)
        print("wrote", name, (OUT / f"{name}.gif").stat().st_size // 1024, "KiB")


def capture(term: Terminal, h: Herdr, scene: list[dict], env: dict[str, str]) -> None:
    shots = Shots(term)
    term.wait_for("doesn't depend on the open questions")
    time.sleep(1.5)
    before = term.rich_lines()[0]
    seed(env, scene)
    term.wait_for("❓2 Billing API")
    time.sleep(1)
    after = term.rich_lines()[0]
    markers(term, before, after)
    shots.frame(1.6)

    term.send(PREFIX, 0.3)  # prefix+i: the inbox popup, on the group list
    term.send("i")
    term.wait_for("Open questions")
    time.sleep(1.2)  # list rows arm after 1 s
    shots.png("list")
    shots.frame(2.4)

    term.send("\r")  # into "Invoice storage and caching": its tabs and its Submit
    term.wait_for("Which database")
    time.sleep(0.8)
    shots.png("group")
    shots.frame(1.6)
    term.send("2")  # jump to SQLite, Enter picks it and moves on to the next tab
    term.send("\r")
    term.wait_for("Which caches")
    time.sleep(0.8)
    shots.frame(1.2)
    term.send(" ")  # redis
    cursor_to(term, "CDN")
    term.send(" ", 0.6)
    shots.frame(1.4)
    cursor_to(term, "Submit")  # multi-select: picks are saved by its Submit row
    term.send("\r")
    term.wait_for("Review your answers")
    time.sleep(0.8)
    shots.png("submit")
    shots.frame(2.2)
    term.send("\r")  # Submit answers: delivered into the Billing API agent
    term.wait_gone("❓2 Billing API")
    time.sleep(1.5)
    shots.frame(2.2)

    wait_toasts(term)
    cursor_to(term, "Merge mode")  # the lone question with previews
    term.send("\r")
    term.wait_for("Merge PR #42")
    time.sleep(0.8)
    shots.png("single")

    term.send("\x1b", 0.8)  # back to the list, then close the popup
    term.send("\x1b", 1.0)
    term.wait_gone("Open questions")
    term.wait_for("[inbox] answered")
    time.sleep(1)
    shots.png("answered")
    shots.frame(3.5)
    shots.gif("answering")


def cursor_to(term: Terminal, label: str, tries: int = 8) -> None:
    """Press j until the ❯ cursor row shows ``label``."""
    for _ in range(tries):
        if any("❯" in line and label in line for line in term.text().splitlines()):
            return
        term.send("j", 0.3)
    raise TimeoutError(f"cursor never reached {label!r}:\n{term.text()}")


def wait_toasts(term: Terminal, timeout: float = 15) -> None:
    """Textual's notifications ("Sending Q2…", "Sent Q2, Q3") fade after a few seconds."""
    end = time.time() + timeout
    while time.time() < end and any(w in term.text() for w in ("Sending Q", "Sent Q")):
        time.sleep(0.3)


def markers(term: Terminal, before, after) -> None:
    """The tab bar before and after the questions were filed, one above the other."""
    from rich.text import Text

    start, width = 26, 118
    label = lambda s: Text(f" {s:<7}", style="#a6adc8 on #1e1e2e")  # noqa: E731
    lines = [Text(" ", style="on #1e1e2e"), label("before") + before[start:start + width],
             Text(" ", style="on #1e1e2e"), label("after") + after[start:start + width], Text(" ", style="on #1e1e2e")]
    raw = DEMO / "raw" / "tab-marker.png"
    term.png(raw, "herdr", 1400, lines, width + 8)
    frame_file(raw, OUT / "tab-marker.png", pad=60)
    print("wrote tab-marker")


def main() -> None:
    env = prepare()
    h = Herdr(env)
    subprocess.run(["herdr", "plugin", "link", str(ROOT)], env=env, check=True, capture_output=True)
    term = Terminal(["herdr"], env, COLS, ROWS, cwd=str(HOME / "acme"))
    try:
        wait_socket(h)
        scene = build_scene(h)
        if os.environ.get("DEMO_PAUSE"):
            raise RuntimeError("paused after build_scene (DEMO_PAUSE)")
        capture(term, h, scene, env)
    except Exception:
        if os.environ.get("DEMO_DEBUG"):  # keep the isolated herdr up to poke at; touch DEMO/stop to end
            import traceback
            traceback.print_exc()
            (DEMO / "screen.txt").write_text(term.text())
            while not (DEMO / "stop").exists():
                time.sleep(0.5)
        raise
    finally:
        h.raw("server", "stop", check=False)
        term.close()
        shutil.rmtree(RUN, ignore_errors=True)


if __name__ == "__main__":
    main()
