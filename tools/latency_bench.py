"""Latency bench against an ISOLATED herdr (own HOME/XDG/socket/DB under .scratch/perf).

    uv run python tools/latency_bench.py <label>     (BENCH_ROOT=<other checkout> to measure that code)

Measures: (A) ask_operator round trip through the real stdio MCP server, idle and with the
tab-label lock held by another process; (B) popup submit-to-close and Esc-mid-send, with a
send that lands at once and one that never lands (15 s verify window); (C) herdr API latency
for an unrelated pane while (B)'s send runs, vs idle. Prints JSON.
"""
import asyncio, fcntl, json, os, shutil, statistics, subprocess, sys, threading, time
from pathlib import Path

HERE_ROOT = Path(__file__).resolve().parents[1]
ROOT = Path(os.environ.get("BENCH_ROOT", HERE_ROOT))  # the code under test (before/after checkout)
BASE = HERE_ROOT / ".scratch" / "perf" / "iso"
RUN = Path("/run/user/%d" % os.getuid()) / "hib"
LABEL = sys.argv[1] if len(sys.argv) > 1 else "run"
TRACE = HERE_ROOT / ".scratch" / "perf" / f"trace-{LABEL}.jsonl"


def setup():
    shutil.rmtree(BASE, ignore_errors=True)
    shutil.rmtree(RUN, ignore_errors=True)
    home = BASE / "home"
    for d in ("home/.config/herdr", "home/.local/state", "home/.cache", "bin", "claude", "proj"):
        (BASE / d).mkdir(parents=True, exist_ok=True)
    RUN.mkdir(mode=0o700, parents=True)
    agent = BASE / "bin" / "claude"
    agent.write_text("#!/usr/bin/python3\n" + (HERE_ROOT / "docs/screenshots/fake_agent.py").read_text().split("\n", 1)[1])
    agent.chmod(0o755)
    (home / ".config/herdr/config.toml").write_text(
        'onboarding = false\n[update]\nversion_check = false\nmanifest_check = false\n'
        '[terminal]\ndefault_shell = "/bin/bash"\n')
    TRACE.unlink(missing_ok=True)
    env = {
        "PATH": f"{BASE / 'bin'}:{os.environ['PATH']}", "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"), "XDG_STATE_HOME": str(home / ".local/state"),
        "XDG_CACHE_HOME": str(home / ".cache"), "XDG_RUNTIME_DIR": str(RUN),
        "HERDR_SOCKET_PATH": str(RUN / "s.sock"), "HERDR_INBOX_DB": str(BASE / "inbox.db"),
        "HERDR_PLUGIN_CONFIG_DIR": str(BASE / "plugin-config"),
        "CLAUDE_CONFIG_DIR": str(BASE / "claude"), "INBOX_TRACE": str(TRACE),
        "HERDR_INBOX_WORKER_IDLE_EXIT": "3", "TERM": "xterm-256color", "LANG": "C.UTF-8",
        "UV_CACHE_DIR": os.environ.get("UV_CACHE_DIR") or str(Path.home() / ".cache" / "uv"),
    }
    (BASE / "plugin-config").mkdir()
    (BASE / "plugin-config" / "config.toml").write_text('tab_prefix = "❓"\nprefix_unnamed_tabs = true\n')
    return env


def h(env, *args, timeout=10):
    out = subprocess.run(["herdr", *args], env=env, capture_output=True, text=True, timeout=timeout)
    return json.loads(out.stdout)["result"] if out.stdout.strip().startswith("{") else out.stdout


def pct(xs):
    xs = sorted(xs)
    return {"n": len(xs), "p50": round(statistics.median(xs), 1), "p95": round(xs[int(len(xs) * .95) - 1], 1),
            "max": round(xs[-1], 1)}


def probe_loop(env, pane, stop, out):
    while not stop.is_set():
        t = time.perf_counter()
        subprocess.run(["herdr", "pane", "get", pane], env=env, capture_output=True, timeout=10)
        out.append((time.perf_counter() - t) * 1000)
        time.sleep(0.02)


def main():
    env = setup()
    server = subprocess.Popen(["herdr", "server"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                              start_new_session=True)
    results = {"label": LABEL}
    try:
        for _ in range(100):
            if (RUN / "s.sock").exists():
                break
            time.sleep(0.1)
        time.sleep(0.5)
        ws = h(env, "workspace", "create", "--cwd", str(BASE / "proj"), "--label", "bench")
        wid = ws["workspace"]["workspace_id"]
        t1 = h(env, "tab", "create", "--workspace", wid, "--label", "Agent", "--cwd", str(BASE / "proj"))
        t2 = h(env, "tab", "create", "--workspace", wid, "--label", "Other", "--cwd", str(BASE / "proj"))
        pa, ta = t1["root_pane"]["pane_id"], t1["tab"]["tab_id"]
        pb = t2["root_pane"]["pane_id"]
        h(env, "pane", "send-text", pa, "claude billing\n")
        time.sleep(2)
        os.environ.update(env)
        os.environ["HERDR_PANE_ID"] = pa
        os.environ["HERDR_TAB_ID"] = ta
        sys.path.insert(0, str(ROOT / "src"))
        from herdr_inbox import service, transcript, tui
        from herdr_inbox.service import Caller
        from herdr_inbox.store import Store

        # (C0) idle herdr latency
        idle = []
        stop = threading.Event()
        th = threading.Thread(target=probe_loop, args=(env, pb, stop, idle)); th.start()
        time.sleep(3); stop.set(); th.join()
        results["herdr_idle_ms"] = pct(idle)

        # (A) ask_operator through the real MCP server
        async def asks():
            from mcp import Client, StdioServerParameters
            params = StdioServerParameters(command=str(ROOT / ".venv/bin/inbox"), args=["mcp"],
                                           env={**os.environ, "HERDR_PANE_ID": pa, "HERDR_TAB_ID": ta})
            lat, contended = [], []
            async with Client(params, mode="legacy", read_timeout_seconds=60) as client:
                for i in range(6):
                    t = time.perf_counter()
                    await client.call_tool("ask_operator", {"question": f"Bench question {i}?", "options": ["a", "b"]})
                    lat.append((time.perf_counter() - t) * 1000)
                # tab-label lock held by another process (a popup or another ask relabelling)
                lockf = open(str(Path(env["HERDR_INBOX_DB"])) + ".markers.lock", "a+")
                holder = subprocess.Popen([sys.executable, "-c", (
                    "import fcntl,sys,time; f=open(sys.argv[1],'a+'); fcntl.flock(f, fcntl.LOCK_EX);"
                    " print('x', flush=True); time.sleep(4)"), lockf.name], stdout=subprocess.PIPE, text=True)
                holder.stdout.readline()
                for i in range(2):
                    t = time.perf_counter()
                    await client.call_tool("ask_operator", {"question": f"Bench contended {i}?", "options": ["a"]})
                    contended.append((time.perf_counter() - t) * 1000)
                holder.wait()
            return lat, contended
        lat, contended = asyncio.run(asks())
        results["ask_operator_ms"] = pct(lat)
        results["ask_operator_lock_held_ms"] = [round(x) for x in contended]

        # (B) popup submit-to-close, with (C) herdr latency during the send
        async def popup(case):
            store = Store()
            for q in store.list():
                service.close(store, q.id, "dismissed")
            for q in store.failed_deliveries():  # earlier cases' rows: the popup must be empty after
                store.hide_failed(q.id)
            qid = service.ask(store, Caller(pa, ta, "Agent", None, str(BASE / "proj")), f"Popup {case}?",
                              options=["yes", "no"])["id"]
            h(env, "pane", "report-agent", pa, "--source", "readme-demo", "--agent", "claude", "--state", "idle")
            dummy = BASE / "claude" / "never.jsonl"
            dummy.write_text("")
            orig_find, orig_wait = transcript.find, transcript.wait_landed
            transcript.find = lambda *a, **k: dummy
            if case == "lands":
                transcript.wait_landed = lambda *a, **k: True
            during, stop = [], threading.Event()
            th = threading.Thread(target=probe_loop, args=(env, pb, stop, during))
            app = tui.InboxApp(store, focus_pane=pb)
            try:
                async with app.run_test() as pilot:
                    await pilot.pause(0.6)
                    th.start()
                    t0 = time.perf_counter()
                    await pilot.press("enter")  # the lone question: answer option 1, send
                    if case == "esc":
                        await pilot.pause(0.2)
                        t0 = time.perf_counter()
                        await pilot.press("escape")
                    while app.is_running and time.perf_counter() - t0 < 30:
                        await asyncio.sleep(0.01)
                    closed = (time.perf_counter() - t0) * 1000
            finally:
                transcript.find, transcript.wait_landed = orig_find, orig_wait
                time.sleep(1)
                stop.set(); th.join()
            return closed, during
        for case in ("lands", "never_lands", "esc"):
            closed, during = asyncio.run(popup(case))
            results[f"popup_{case}_to_close_ms"] = round(closed)
            results[f"herdr_during_send_{case}_ms"] = pct(during) if during else None
    finally:
        subprocess.run(["herdr", "server", "stop"], env=env, capture_output=True, timeout=10)
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill()
        shutil.rmtree(RUN, ignore_errors=True)
    steps = {}
    for line in TRACE.read_text().splitlines() if TRACE.exists() else []:
        r = json.loads(line)
        steps.setdefault(r["step"], []).append(r["ms"])
    results["trace_max_ms"] = {k: round(max(v), 1) for k, v in sorted(steps.items()) if max(v) >= 16}
    print(json.dumps(results, indent=1))


main()
