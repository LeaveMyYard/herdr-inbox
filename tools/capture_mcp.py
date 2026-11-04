"""Record herdr-inbox's MCP server (the Python one) as goldens for the Rust one (P5-F).

    uv run python tools/capture_mcp.py tests/fixtures/mcp

Each session starts the server (`python -m herdr_inbox.cli mcp`) with every HERDR_* and
CLAUDE_* variable unset, herdr pointed at a path that does not exist, no delivery worker
(HERDR_INBOX_NO_WORKER) and its own database in a temp dir (migrated first, as the worker
leaves it: MCP tools never migrate). It sends the session's requests one line at a time and
records every line the server writes: `{"dir": ..., "msg": ...}`. A request with an id waits
for its reply; `subscriptions/listen` waits for the acknowledgement.

- `claude-code.jsonl`: Claude Code 2.1.302's own requests (`claude-code-2.1.302.requests.jsonl`,
  cut from herdr-agent-kit's fixtures/claude/mcp recording of this server, protocol 2026-07-28).
- `legacy.jsonl`: the 2025-06-18 `initialize` handshake, `tools/list`, an unknown method, an
  unknown tool, `ping`, and calls the schema refuses.
- `outside-herdr.jsonl`: the six tools in a session that is not in a herdr pane.
- `claude-code-calls.jsonl`: Claude Code's requests, then tool calls with its `_meta` (2026-07-28).

Record on Python 3.12, the Python the live MCP server runs: from 3.13 the compiler strips
docstrings' indentation, and three tool descriptions are docstrings.

Never run it against the operator's database: it sets HERDR_INBOX_DB itself, to a temp dir.
"""
import json
import os
import pathlib
import queue
import subprocess
import sys
import tempfile
import threading

ROOT = pathlib.Path(__file__).resolve().parent.parent
NO_HERDR = "/nonexistent/herdr-not-allowed-in-captures"
INIT = [{"jsonrpc": "2.0", "id": 0, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "capture", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"}]


def call(i, name, args):
    return {"jsonrpc": "2.0", "id": i, "method": "tools/call", "params": {"name": name, "arguments": args}}


SESSIONS = {
    "legacy": INIT + [
        {"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
        {"jsonrpc": "2.0", "id": 2, "method": "no/such/method"},
        {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "no_such_tool", "arguments": {}}},
        {"jsonrpc": "2.0", "id": 4, "method": "ping"},
        call(5, "withdraw_alert", {}),
        call(6, "withdraw_alert", {"id": "seven"}),
        call(7, "withdraw_question", {"id": 1}),
        call(8, "alert_operator", {}),
        call(9, "withdraw_question", {}),
        call(10, "ask_operator", {"options": [5]}),
        call(11, "ask_operator", {"options": [{"label": 5, "description": 3}]}),
        call(12, "ask_operator", {"questions": [{"options": [None], "multi_select": "x"}], "header": 3}),
        call(13, "ask_operator", {"question": "q", "header": "[]"}),
        call(14, "ask_operator", {"question": "q", "multi_select": "null"}),
        call(15, "withdraw_alert", {"id": "true"}),
    ],
    "outside-herdr": INIT + [
        call(1, "ask_operator", {"question": "Deploy?"}),
        call(2, "ask_operator", {}),
        call(3, "ask_operator", {"question": "a", "questions": [{"question": "b"}]}),
        call(4, "ask_operator", {"questions": [{"question": f"q{i}"} for i in range(5)]}),
        call(5, "ask_operator", {"question": "many?", "options": [str(i) for i in range(10)]}),
        call(6, "check_answers", {}),
        call(7, "list_my_questions", {}),
        call(8, "list_my_questions", {"include_closed": True}),
        call(9, "withdraw_question", {"id": 1, "reason": "done"}),
        call(10, "alert_operator", {"title": "Look"}),
        call(11, "withdraw_alert", {"id": 1}),
        # FastMCP reads JSON text in a non-`str` argument first (pre_parse_json)
        call(12, "ask_operator", {"questions": json.dumps([{"question": f"q{i}"} for i in range(5)])}),
        call(13, "ask_operator", {"question": "many?", "options": json.dumps([str(i) for i in range(10)])}),
        call(14, "ask_operator", {"question": "null"}),
    ],
}


def run(requests, env, out):
    p = subprocess.Popen([sys.executable, "-m", "herdr_inbox.cli", "mcp"], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, text=True, env=env, cwd=ROOT)
    lines: queue.Queue = queue.Queue()
    threading.Thread(target=lambda: [lines.put(json.loads(x)) for x in p.stdout], daemon=True).start()
    rec = []
    for r in requests:
        rec.append({"dir": "client->server", "msg": r})
        p.stdin.write(json.dumps(r, ensure_ascii=False) + "\n")
        p.stdin.flush()
        if "id" not in r:
            continue
        listen = r["method"] == "subscriptions/listen"
        while True:
            m = lines.get(timeout=60)
            rec.append({"dir": "server->client", "msg": m})
            if listen and m.get("method") == "notifications/subscriptions/acknowledged":
                break
            if m.get("id") == r["id"]:
                break
    p.stdin.close()
    p.wait(timeout=30)
    while not lines.empty():
        rec.append({"dir": "server->client", "msg": lines.get()})
    with open(out, "w") as fh:
        for x in rec:
            fh.write(json.dumps(x, ensure_ascii=False) + "\n")
    return rec


def migrated(db: pathlib.Path, env: dict) -> None:
    """The database as the worker leaves it (only the worker and the popup migrate)."""
    code = "from herdr_inbox.store import Store; Store(migrate=True).close()"
    subprocess.run([sys.executable, "-c", code], env=env, cwd=ROOT, check=True)
    assert db.exists()


def main(out_dir: str) -> None:
    out = pathlib.Path(out_dir)
    base = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "CLAUDE_"))}
    claude = [json.loads(x) for x in (out / "claude-code-2.1.302.requests.jsonl").read_text().splitlines()]
    meta = claude[-1]["params"]["_meta"]  # the recorded tools/list request's
    calls = [{**c, "params": {**c["params"], "_meta": meta}} for c in (
        call(100, "ask_operator", {}), call(101, "withdraw_alert", {"id": "x"}), call(102, "no_such_tool", {}))]
    sessions = [("claude-code", claude), ("claude-code-calls", claude + calls), *SESSIONS.items()]
    for name, reqs in sessions:
        with tempfile.TemporaryDirectory() as tmp:
            db = pathlib.Path(tmp) / "inbox.db"
            env = {**base, "HERDR_BIN_PATH": NO_HERDR, "HERDR_INBOX_DB": str(db), "HERDR_INBOX_NO_WORKER": "1",
                   "XDG_STATE_HOME": tmp, "XDG_CONFIG_HOME": tmp, "HOME": tmp}
            migrated(db, env)
            run(reqs, env, out / f"{name}.jsonl")


if __name__ == "__main__":
    main(sys.argv[1])
