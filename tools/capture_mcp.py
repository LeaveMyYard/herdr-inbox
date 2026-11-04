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
- `herdr-*.jsonl`: `ask_operator` and `check_answers` in a herdr pane, herdr being the test suite's
  fake herdr CLI (tests/conftest.py `FAKE_HERDR`): the pane's identity, a moved pane, the session
  from the environment, herdr unreachable. `{"dir": "action"}` lines are done to the database
  between two requests (the operator answers); the session ends with the herdr calls the server
  made in a fixed order (`"dir": "herdr"`: agent lists, notifications, pane lookups), the
  markers' end state (`"dir": "markers"`: each pane's last count token, each tab's last label) and the questions and alerts as stored (`"dir": "store"`, `"alerts"`; cwd and herdr server
  written as `<cwd>` and `<server>`).

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


S = "aaaaaaaa-0000-4000-8000-00000000000a"
ASK = [
    call(1, "ask_operator", {"question": "  Deploy to prod?\n", "asked_by": "release agent", "context": "e2e",
                             "header": "  Release gate, long", "options": ["yes", {"label": "no", "description": "wait",
                                                                                  "x": 1}]}),
    call(2, "ask_operator", {"question": "deploy to PROD"}),
    call(3, "check_answers", {}),
    {"action": {"answer": [1, "no, wait for Monday"]}},
    call(4, "check_answers", {}),
    call(5, "check_answers", {}),
    call(6, "ask_operator", {"question": "Deploy to prod?"}),
    call(7, "ask_operator", {"title": "Rollout settings \u2713", "questions": [
        {"question": "Which region?", "options": ["eu", "us"], "header": "Region"},
        {"question": "D\u00e9ploy \u2713 \u2014 na\u00efve?", "multi_select": True, "options": ["a", "b"]},
        {"question": "which REGION?"}]}),
    call(8, "ask_operator", {"question": "Bell \u0007 and escape \u001b[31m?\nsecond line"}),
]
HERDR_SESSIONS = {
    # the pane, its tab (without our "❓2 " mark) and its session, all from herdr
    "herdr-pane": ({"FAKE_HERDR_TABS_JSON": json.dumps({"w1:t1": "\u2753" + "2 my-tab"})}, INIT + ASK),
    # herdr no longer knows w1:p1 (a moved pane): the one pane running this conversation
    "herdr-moved": ({"FAKE_HERDR_GONE_PANES": "w1:p1", "CLAUDE_CODE_SESSION_ID": S,
                     "FAKE_HERDR_AGENTS": json.dumps([{"pane_id": "w1:p3", "agent_session": {"value": "other"}},
                                                      {"pane_id": "w1:p2", "agent_session": {"value": S}}])},
                    INIT + [call(1, "ask_operator", {"question": "Where am I?"})]),
    # herdr knows the pane but not its session
    "herdr-env-session": ({"FAKE_HERDR_SESSION": "", "CLAUDE_CODE_SESSION_ID": "claude-sess"},
                          INIT + [call(1, "ask_operator", {"question": "Which session?"}), call(2, "check_answers", {})]),
    # the caller's own questions and alerts: list, withdraw, raise, acknowledge
    "herdr-tools": ({}, INIT + [
        call(1, "ask_operator", {"question": "Keep me?"}),
        call(2, "ask_operator", {"question": "Withdraw me?"}),
        call(3, "list_my_questions", {}),
        call(4, "withdraw_question", {"id": 2, "reason": "found it in terraform"}),
        call(5, "withdraw_question", {"id": 2, "reason": "again"}),
        call(6, "withdraw_question", {"id": 99, "reason": "not mine"}),
        {"action": {"answer": [1, "kept"]}},
        call(7, "list_my_questions", {}),
        call(8, "list_my_questions", {"include_closed": True}),
        call(9, "alert_operator", {"title": "  Deploy\n needs   eyes ", "message": "  see the log  ",
                                   "severity": "urgent", "asked_by": "deployer"}),
        call(10, "alert_operator", {"title": "Deploy needs eyes"}),
        call(11, "alert_operator", {"title": "   "}),
        call(12, "alert_operator", {"title": "Odd severity", "severity": "bogus"}),
        call(13, "list_my_questions", {}),
        call(14, "withdraw_alert", {"id": 2}),
        call(15, "withdraw_alert", {"id": 2}),
        call(16, "withdraw_alert", {"id": 99}),
        {"action": {"ack_alert": 1}},
        call(17, "check_answers", {}),
        call(18, "check_answers", {}),
        call(19, "list_my_questions", {"include_closed": True}),
        # a last marker refresh after the operator's actions: the markers' end state does not
        # depend on whether an earlier refresh ran before or after an action
        call(20, "ask_operator", {"question": "Question Title 9001?"}),
    ]),
    # herdr cannot be asked at all
    "herdr-down": ({"FAKE_HERDR_FAIL": "pane get,tab get,agent list,notification show,pane report-metadata,pane list",
                    "CLAUDE_CODE_SESSION_ID": S}, INIT + [call(1, "ask_operator", {"question": "Anyone there?"})]),
}
QUESTION_COLUMNS = ("id, pane_id, tab_id, tab_label, session_id, cwd, asked_by, question, options, context, norm, "
                    "status, answer, repeat_count, acknowledged_at IS NOT NULL, delivery_error, header, multi_select, "
                    "group_id, options_json, herdr_server, group_title")


def act(action: dict, env: dict) -> None:
    if "ack_alert" in action:
        # the operator's acknowledgement as the store records it (alerts._close without the
        # markers refresh, which the popup does: not this server's work)
        code = ("from herdr_inbox.store import Store, now; s = Store(); s.db.execute(\"UPDATE alerts SET "
                "status = 'acked', acked_at = ? WHERE id = ? AND status = 'open'\", "
                f"(now(), {action['ack_alert']})); s.close()")
    else:
        qid, text = action["answer"]
        code = f"from herdr_inbox.store import Store; s = Store(); s.set_answer({qid}, {text!r}); s.close()"
    subprocess.run([sys.executable, "-c", code], env=env, cwd=ROOT, check=True)


ALERT_SQL = ("SELECT id, kind, severity, title, message, pane_id, tab_id, tab_label, session_id, asked_by, "
             "herdr_server, status, acked_at IS NOT NULL, closed_at IS NOT NULL, reported_at IS NOT NULL "
             "FROM alerts ORDER BY id")


def stored(env: dict, sql: str | None = None) -> list:
    code = ("import json, sqlite3, sys; db = sqlite3.connect(sys.argv[1]); "
            "print(json.dumps([list(r) for r in db.execute(sys.argv[2])]))")
    questions = sql is None
    sql = sql or (f"SELECT {QUESTION_COLUMNS} FROM questions JOIN question_ext USING (id) "
                  "LEFT JOIN question_groups USING (group_id) ORDER BY id").replace("group_title", "title")
    out = subprocess.run([sys.executable, "-c", code, env["HERDR_INBOX_DB"], sql], capture_output=True, text=True,
                         check=True).stdout
    rows = json.loads(out)
    for r in rows:
        if questions:
            r[5] = "<cwd>" if r[5] == str(ROOT) else r[5]
            r[20] = "<server>" if r[20] else r[20]
        else:
            r[10] = "<server>" if r[10] else r[10]
    return rows


def run(requests, env, out, herdr_log=None):
    p = subprocess.Popen([sys.executable, "-m", "herdr_inbox.cli", "mcp"], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, text=True, env=env, cwd=ROOT)
    lines: queue.Queue = queue.Queue()
    threading.Thread(target=lambda: [lines.put(json.loads(x)) for x in p.stdout], daemon=True).start()
    rec = []
    for r in requests:
        if "action" in r:
            rec.append({"dir": "action", "msg": r["action"]})
            act(r["action"], env)
            continue
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
    if herdr_log is not None:
        # only the calls made in a fixed order: the tool's own pane lookups, and the notifications
        # (one background thread raises them in turn). The marker job's calls interleave with the
        # tool's however the threads ran, so they are left out, and each kind is kept on its own.
        kinds = (["agent", "list"], ["notification", "show"], ["pane", "get"])
        calls = [json.loads(x) for x in herdr_log.read_text().splitlines() if x]
        rec += [{"dir": "herdr", "msg": c} for k in kinds for c in calls if c[:2] == k]
        # the marker job's end state: each pane's last count token, each tab's last rename
        tokens, labels = {}, {}
        for c in calls:
            if c[:2] == ["pane", "report-metadata"]:
                tokens[c[2]] = c[-1] if "--token" in c else None
            if c[:2] == ["tab", "rename"]:
                labels[c[2]] = c[3]
        rec.append({"dir": "markers", "msg": {"tokens": tokens, "labels": labels}})
        rec.append({"dir": "store", "msg": stored(env)})
        rec.append({"dir": "alerts", "msg": stored(env, ALERT_SQL)})
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
    sys.path.insert(0, str(ROOT))
    from tests.conftest import COMPOSER, FAKE_HERDR  # the suite's fake herdr CLI

    for name, (extra, reqs) in HERDR_SESSIONS.items():
        with tempfile.TemporaryDirectory() as tmp:
            db, fake, log = pathlib.Path(tmp) / "inbox.db", pathlib.Path(tmp) / "herdr", pathlib.Path(tmp) / "calls"
            fake.write_text(FAKE_HERDR.format(python=sys.executable, composer_empty=str(COMPOSER / "empty.ansi")))
            fake.chmod(0o755)
            log.touch()
            extra = dict(extra)
            if "FAKE_HERDR_TABS_JSON" in extra:
                (pathlib.Path(tmp) / "tabs.json").write_text(extra.pop("FAKE_HERDR_TABS_JSON"))
                extra["FAKE_HERDR_TABS"] = str(pathlib.Path(tmp) / "tabs.json")
            env = {**base, "HERDR_BIN_PATH": str(fake), "FAKE_HERDR_LOG": str(log), "HERDR_PANE_ID": "w1:p1",
                   "HERDR_INBOX_DB": str(db), "HERDR_INBOX_NO_WORKER": "1", "XDG_STATE_HOME": tmp,
                   "XDG_CONFIG_HOME": tmp, "HOME": tmp, "CLAUDE_CONFIG_DIR": str(pathlib.Path(tmp) / "claude"),
                   **extra}
            migrated(db, env)
            log.write_text("")
            run(reqs, env, out / f"{name}.jsonl", herdr_log=log)


if __name__ == "__main__":
    main(sys.argv[1])
