#!/usr/bin/env python3
"""Each mutation of a copy of the fixtures must make validate.py fail. usage: mutate.py ROOT"""
import json, shutil, subprocess, sys, tempfile
from pathlib import Path
WT = Path(sys.argv[1]); HERE = Path(__file__).parent
E, C = "tests/fixtures/app-feed/every-kind.json", "tests/fixtures/app-commands/commands.json"
def ent(d, key): return next(e for e in d["entries"] if e["key"] == key)
def case(d, n): return next(c for c in d["cases"] if c["name"] == n)
def swap(d, a, b):
    es = d["entries"]; i, j = es.index(ent(d, a)), es.index(ent(d, b)); es[i], es[j] = es[j], es[i]
M = [
 ("alerts by id", E, lambda d: swap(d, "alert:4", "alert:5")),
 ("groups out of order", E, lambda d: swap(d, "group:12", "group:14")),
 ("failed out of order", E, lambda d: swap(d, "failed:7", "failed:8")),
 ("question mark below a group", E, lambda d: d["new_since"].update(question_id=12)),
 ("alert mark below an alert", E, lambda d: d["new_since"].update(alert_id=1)),
 ("wait disagrees with reason", E, lambda d: ent(d, "failed:8")["delivery"].update(wait="panel")),
 ("junk result token", C, lambda d: case(d, "draft")["stdout"].update(token="0" * 32)),
 ("null token on a row that stays", C, lambda d: case(d, "draft")["stdout"].update(token=None)),
 ("token on a row that went", C, lambda d: case(d, "answer-lone")["stdout"].update(token="0" * 32)),
 ("wrong done", C, lambda d: case(d, "dismiss-group")["stdout"].update(done=[14])),
 ("wrong skipped", C, lambda d: case(d, "answer-skips-no-draft")["stdout"].update(skipped=[])),
 ("wrong closed word", C, lambda d: case(d, "stale-acked")["stdout"].update(closed="cleared")),
 ("non-canonical id accepted", C, lambda d: case(d, "draft")["argv"].__setitem__(case(d, "draft")["argv"].index("16"), "016")),
 ("core row forced", C, lambda d: case(d, "retry-core")["effects"]["sender"].append("--force")),
 ("TUI row not forced", C, lambda d: case(d, "retry-tui")["effects"]["sender"].remove("--force")),
 ("default socket for another server", C, lambda d: case(d, "retry-other-server")["effects"].update(sender_socket=None)),
 ("queued with no host", C, lambda d: case(d, "retry-held")["stdout"].update(delivery="queued")),
 ("held starts a sender", C, lambda d: case(d, "retry-held")["effects"].update(sender=["deliver", "10", "--force"])),
 ("held skips the worker", C, lambda d: case(d, "retry-held").update(effects=None)),
 ("unproven counts as gone", C, lambda d: case(d, "answer-unproven")["stdout"].update(delivery="held")),
 ("no session forced", C, lambda d: case(d, "retry-held-no-session")["stdout"].update(delivery="queued")),
 ("core before B3 queued", C, lambda d: case(d, "answer-held-core-before-b3")["stdout"].update(delivery="queued")),
 ("core row with a socket", C, lambda d: case(d, "retry-core").update(server="/run/x.sock")),
 ("core sender sets a socket", C, lambda d: case(d, "retry-core")["effects"].update(sender_socket="herdr-core")),
 ("claim case refused", C, lambda d: case(d, "stop-showing-during-claim").update(exit=1, stdout={"v":1,"ok":False,"verb":"stop-showing","entry":"failed:30","code":"stale","message":"x","closed":None})),
 ("replay_of dropped", C, lambda d: case(d, "retry-replay").update(replay_of=None)),
 ("detail echoed", C, lambda d: case(d, "invalid-empty-answer").update(argv=case(d, "invalid-empty-answer")["argv"][:-2] + ['{"choices":["Option Z"],"notes":{},"text":"secret"}', "--json"]) or case(d, "invalid-empty-answer")["stdout"].update(message="secret")),
 ("server field removed", C, lambda d: case(d, "ack").pop("server")),
 ("host field junk", C, lambda d: case(d, "ack").update(host="maybe")),
 ("show cuts", C, lambda d: case(d, "show")["stdout"]["row"]["questions"][0].update(question="x")),
 ("show changes the token", C, lambda d: case(d, "show")["stdout"]["row"].update(token="0" * 32)),
 ("cut with nothing at a limit", E, lambda d: ent(d, "group:12")["questions"][0].update(cut=True)),
 ("replay succeeds", C, lambda d: case(d, "retry-replay").update(exit=0, stdout=case(d, "retry-tui")["stdout"])),
 ("retry does not move the token", C, lambda d: case(d, "retry-tui")["stdout"].update(token=ent(json.load(open(WT / E)), "failed:8")["token"])),
 ("blocked alert on core", E, lambda d: ent(d, "alert:6")["asker"].update(host="herdr-core", pane=None, terminal="t9")),
 ("auto_retry with no pane and no terminal", E, lambda d: ent(d, "failed:37")["delivery"].update(auto_retry=True)),
 ("a refusal starts a worker", C, lambda d: case(d, "busy").update(effects={"ensure_worker": None, "sender": None, "sender_socket": None})),
 ("not_migrated starts nothing", C, lambda d: case(d, "not-migrated").update(effects=None)),
 ("an unknown detail field accepted", C, lambda d: case(d, "usage-unknown-detail-field").update(exit=1, stdout={"v":1,"ok":False,"verb":"answer","entry":"group:12","code":"invalid","message":"x","closed":None})),
 ("a wait word gone", E, lambda d: d["entries"].remove(ent(d, "failed:34"))),
 ("show starts something", C, lambda d: case(d, "show").update(effects={"ensure_worker": None, "sender": None, "sender_socket": None})),
 ("show takes a token", C, lambda d: case(d, "show")["argv"].extend(["--token", "0" * 32])),
 ("show without json", C, lambda d: case(d, "show")["argv"].remove("--json")),
 ("a left-out id published", "tests/fixtures/app-feed/partial.json", lambda d: d["omitted"]["question_ids"].__setitem__(0, 12)),
]
bad = 0
for name, path, fn in M:
    with tempfile.TemporaryDirectory() as t:
        t = Path(t)
        shutil.copytree(WT / "docs/protocol", t / "docs/protocol")
        for sub in ("app-feed", "app-commands"):
            shutil.copytree(WT / "tests/fixtures" / sub, t / "tests/fixtures" / sub)
        d = json.loads((t / path).read_text()); fn(d); (t / path).write_text(json.dumps(d, ensure_ascii=False))
        r = subprocess.run([sys.executable, HERE / "validate.py", t], capture_output=True, text=True)
        if r.returncode == 0:
            bad += 1; print("SURVIVED", name)
print(f"mutations {len(M)}, survived {bad}")
sys.exit(1 if bad else 0)
