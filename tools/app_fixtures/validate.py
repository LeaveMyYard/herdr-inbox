#!/usr/bin/env python3
"""Checks tests/fixtures/app-feed and app-commands against the tables of docs/protocol/app-*-v1.md, and
replays every command case against its world, so the documents and their examples cannot drift apart.
That the code reproduces the same fixtures is tests/app/. usage: validate.py ROOT"""
import copy, hashlib, json, re, sys
from pathlib import Path

ROOT = Path(sys.argv[1])
feed_md = (ROOT / "docs/protocol/app-feed-v1.md").read_text()
cmd_md = (ROOT / "docs/protocol/app-commands-v1.md").read_text()
errors = []


def err(where, msg):
    errors.append(f"{where}: {msg}")


def table_after(md, heading):
    i = md.index(heading)
    m = re.search(r"^\| Field \| Type \| Meaning \|\n\|---\|---\|---\|\n((?:\|.*\n)+)", md[i:], re.M)
    out = {}
    for row in m.group(1).splitlines():
        cells = [c.strip() for c in re.split(r"(?<!\\)\|", row)[1:-1]]
        out[cells[0].strip("`")] = cells[1].strip("`")
    return out


T = {n: table_after(feed_md, h) for n, h in {
    "Feed": "## Feed\n", "Omitted": "### Omitted", "NewSince": "### NewSince", "Entry": "## Entries\n",
    "group": "### `group`", "Question": "### Question", "Option": "### Option", "Detail": "### Detail",
    "Asker": "### Asker", "failed": "### `failed`", "Delivery": "### Delivery\n\n| Field", "alert": "### `alert`",
    "Machine": "Machine:\n", "remote_group": "`remote_group`:\n", "remote_outbox": "`remote_outbox`:\n",
    "remote_problem": "`remote_problem`:\n", "remote_alert": "`remote_alert`:\n"}.items()}
T["_ok"] = table_after(cmd_md, "### Success")
T["_no"] = table_after(cmd_md, "### Refusal")
T["_show"] = table_after(cmd_md, "### Showing one question in full")

KINDS = {k: (key.replace("\\|", "|"), a) for k, key, a in
         re.findall(r"^\| `(\w+)` \| `([^`]+)` \| .* \| (.*) \|$", feed_md, re.M) if k in T}
assert set(KINDS) == {"group", "failed", "alert", "remote_group", "remote_outbox", "remote_problem", "remote_alert"}, KINDS

# the frozen token list, read from the document
tok_md = re.sub(r"\s*\n(?!\|)\s*", " ", feed_md[feed_md.index("## Tokens"):feed_md.index("## What the reader derives")])
TOKEN_F = {k: re.findall(r"`(\w+)`", v) for k, v in re.findall(r"\| `(group|failed|alert)` \| (`key`[^|]*) \|", tok_md)}
assert set(TOKEN_F) == {"group", "failed", "alert"}, TOKEN_F
inner = re.search(r"`asker` by (.*?); each question by (.*?); each option by (.*?); a draft by (.*?)\.", tok_md, re.S)
ASKER_F, Q_F, OPT_F, DET_F = (re.findall(r"`(\w+)`", g) for g in inner.groups())
LIMITS = {"question": 16384, "context": 16384, "answer": 16384, "preview": 4096, "description": 1024}
for row, n in (("`question`, `context`, `answer` | 16 KiB", 1), ("`preview` | 4 KiB", 1), ("`description` | 1 KiB", 1)):
    assert row in feed_md, row

TIME = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\+00:00$")
CANON = re.compile(r"^[1-9][0-9]{0,15}$")
SEVERITY_RANK = {"urgent": 0, "warning": 1}  # anything else comes last, as in open_alerts
STATES = {"unattempted", "claimed", "pending", "agent_gone", "failed"}
WAITS = {"busy_idle", "requeued", "panel", "busy", "blocked", "operator_here", "other"}
ORDER = ["alert", "remote_alert", "group", "failed", "remote_group", "remote_outbox", "remote_problem"]
BUSY_WAIT_IDLE = "busy: typed while the agent was busy but never seen; waiting for it to be idle"
PANEL = "blocked: panel: the agent's background-task panel has focus"


def classify(reason):
    """The document's first-fit table."""
    if reason == BUSY_WAIT_IDLE:
        return "busy_idle"
    if reason.startswith("requeued:"):
        return "requeued"
    if reason == PANEL:
        return "panel"
    if reason.startswith("busy:"):
        return "busy"
    if reason.startswith("blocked:"):
        return "blocked"
    if reason == "waiting: the operator is in that pane":
        return "operator_here"
    return "other"


def pick(o, fields):
    return {k: o[k] for k in fields}


def token(e):
    x = pick(e, TOKEN_F[e["kind"]])
    x["asker"] = pick(x["asker"], ASKER_F)
    if "questions" in x:
        qs = []
        for q in x["questions"]:
            y = pick(q, Q_F)
            y["options"] = [pick(o, OPT_F) for o in y["options"]]
            y["draft"] = y["draft"] and pick(y["draft"], DET_F)
            qs.append(y)
        x["questions"] = qs
    c = json.dumps(x, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(c.encode()).hexdigest()[:32]


def check(v, ty, where, strict):
    if ty.startswith("?"):
        return v is None or check(v, ty[1:], where, strict)
    if ty.startswith("["):
        if not isinstance(v, list):
            return err(where, f"not a list: {v!r}")
        for i, x in enumerate(v):
            check(x, ty[1:-1], f"{where}[{i}]", strict)
        return
    if ty == "Entry":
        return
    if ty in T:
        return obj(v, ty, where, strict=strict)
    good = {"integer": lambda: type(v) is int and abs(v) <= 9007199254740991,
            "number": lambda: type(v) in (int, float),
            "bool": lambda: type(v) is bool,
            "string": lambda: type(v) is str,
            "time": lambda: type(v) is str and TIME.match(v),
            "object": lambda: type(v) is dict and all(type(x) is str for x in v.values())}[ty]()
    if not good:
        err(where, f"not {ty}: {v!r}")


def obj(v, name, where, extra=None, strict=True):
    if type(v) is not dict:
        return err(where, f"not an object: {v!r}")
    want = {**(extra or {}), **T[name]}
    missing, surplus = set(want) - set(v), set(v) - set(want)
    if missing or (surplus and strict):
        err(where, f"{name} fields differ: extra {surplus}, missing {missing}")
    for k, ty in want.items():
        if k in v:
            check(v[k], ty, f"{where}.{k}", strict)


def asker(a, w):
    h = a["host"]
    if h not in ("herdr-tui", "herdr-core", "unknown"):
        err(w, f"host {h}")
    if (a["pane"] is not None) != (h == "herdr-tui"):
        err(w, "pane set iff herdr-tui")
    if (a["terminal"] is not None) != (h == "herdr-core"):
        err(w, "terminal set iff herdr-core")


def limits(o, w, whole=False):
    near = False
    for k, n in LIMITS.items():
        val = o.get(k)
        if type(val) is str:
            size = len(val.encode())
            if size > n and not whole:
                err(w, f"{k} over its limit")
            near = near or n - 3 <= size <= n  # a cut stops at a character boundary
    for x in o.get("options", []):
        limits(x, w, whole)
    if o.get("cut") and not near:
        err(w, "cut, with nothing at a limit")


def check_feed(f, name):
    strict = name != "unknown-vocabulary.json"
    obj(f, "Feed", name, strict=strict)
    if f["v"] != 1:
        err(name, "v")
    om = f["omitted"]
    if not f["ok"] and (f["entries"] or not f["errors"]):
        err(name, "ok false: no entries, an error")
    if any(om.values()) and not f["errors"]:
        err(name, "omitted needs an errors line")
    if om["questions"] < om["groups"]:
        err(name, "a group has a question")
    if len(om["question_ids"]) != min(om["questions"], 1000) or om["question_ids"] != sorted(om["question_ids"]):
        err(name, "question_ids")
    pos, keys, last, top = -1, set(), {}, {}
    for e in f["entries"]:
        w = f"{name}:{e.get('key')}"
        k = e["kind"]
        if k not in KINDS:
            if strict:
                err(w, "unknown kind")
            elif e["token"] is not None or e["actions"]:
                err(w, "an unknown kind offers nothing")
            continue
        obj(e, k, w, T["Entry"], strict)
        key, acts = KINDS[k]
        if not re.fullmatch(re.sub(r"<[^>]*>", ".+", key.replace("|", r"\|")), e["key"]):
            err(w, f"key does not match {key}")
        if e["key"] in keys:
            err(w, "duplicate key")
        keys.add(e["key"])
        want = re.findall(r"`([a-z-]+)`", acts)
        if e["actions"] != want:
            err(w, f"actions {e['actions']} != {want}")
        if bool(want) != (e["token"] is not None):
            err(w, "token iff actions")
        if e["token"] is not None and e["token"] != token(e):
            err(w, "token is not the doc's hash")
        if strict and ORDER.index(k) < pos:
            err(w, "kind order")
        pos = ORDER.index(k)
        sort = {"group": lambda: (e["questions"][0]["id"],), "failed": lambda: (e["question_id"],),
                "alert": lambda: (SEVERITY_RANK.get(e["severity"], 2), e["id"])}.get(k)
        if sort:
            if sort() <= last.get(k, (0,)):
                err(w, "order within the kind")
            last[k] = sort()
            top[k] = max(top.get(k, 0), sort()[-1])
        if "asker" in e and strict:
            asker(e["asker"], w)
        limits(e, w)
        if k == "group":
            ids = [x["id"] for x in e["questions"]]
            if not ids or ids != sorted(ids) or e["group"] > ids[0]:
                err(w, "group ids")
        for x in e.get("questions", []):
            limits(x, w)
            labels = [o["label"] for o in x["options"]]
            if len(labels) > 9 or (x["header"] and len(x["header"]) > 12):
                err(w, "limits")
            d = x["draft"]
            if d:
                if not set(d["choices"]) <= set(labels) or not set(d["notes"]) <= set(d["choices"]):
                    err(w, "draft labels")
                if d["choices"] != [l for l in labels if l in d["choices"]]:
                    err(w, "draft order")
                if not x["multi_select"] and len(d["choices"]) > 1:
                    err(w, "draft choices")
        if k == "failed" and strict:
            d = e["delivery"]
            if d["state"] not in STATES:
                err(w, "state")
            if (d["wait"] is not None) != (d["state"] == "pending"):
                err(w, "wait only when pending")
            if d["state"] == "pending" and d["wait"] != classify(d["reason"]):
                err(w, f"wait {d['wait']} does not fit its reason")
            if (d["reason"] is None) != (d["state"] in ("unattempted", "claimed")):
                err(w, "reason")
            if d["state"] == "agent_gone" and d["auto_retry"]:
                err(w, "gone retries")
            if e["asker"]["pane"] is None and d["auto_retry"]:
                err(w, "no pane, no worker retries")
        if k == "alert" and strict:
            if e["alert_kind"] not in ("agent", "blocked") or e["severity"] not in ("urgent", "warning", "info"):
                err(w, "vocab")
            if e["asker"]["cwd"] is not None:
                err(w, "alert cwd")
            if e["alert_kind"] == "blocked" and e["asker"]["host"] != "herdr-tui":
                err(w, "the blocked scan is TUI-only in v1")
    published = {x["id"] for e in f["entries"] if e.get("kind") == "group" for x in e["questions"]}
    if published & set(om["question_ids"]):
        err(name, "a left-out id names a published question")
    ns = f["new_since"]
    if om["question_ids"] and om["question_ids"][-1] > ns["question_id"]:
        err(name, "question mark below a left-out id")
    if f["ok"] and top.get("group", 0) > ns["question_id"]:
        err(name, "question mark below a published group")
    if f["ok"] and top.get("alert", 0) > ns["alert_id"]:
        err(name, "alert mark below a published alert")


feeds = {p.name: json.loads(p.read_text()) for p in sorted((ROOT / "tests/fixtures/app-feed").glob("*.json"))}
for n, f in feeds.items():
    check_feed(f, n)
    if (ROOT / "tests/fixtures/app-feed" / n).stat().st_size > 1048576:
        err(n, "over 1 MiB")

ek = feeds["every-kind.json"]["entries"]
kinds = {e["kind"] for e in ek}
if kinds != set(KINDS):
    err("every-kind", f"kinds {set(KINDS)-kinds} missing")
for kind in ("group", "failed", "alert"):
    hosts = {e["asker"]["host"] for e in ek if e["kind"] == kind}
    if not {"herdr-tui", "herdr-core"} <= hosts:
        err("every-kind", f"{kind} hosts {hosts}")
if "unknown" not in {e["asker"]["host"] for e in ek if e["kind"] == "failed"}:
    err("every-kind", "no failed row with an unknown host")
fd = [e["delivery"] for e in ek if e["kind"] == "failed"]
if {d["state"] for d in fd} != STATES:
    err("every-kind", "delivery states")
if {d["wait"] for d in fd} - {None} != WAITS:
    err("every-kind", f"waits missing {WAITS - {d['wait'] for d in fd}}")
if sum(d["state"] == "agent_gone" for d in fd) < 2:
    err("every-kind", "two gone answers make the gone row")
if {e["alert_kind"] for e in ek if e["kind"] == "alert"} != {"agent", "blocked"}:
    err("every-kind", "alert kinds")
if not any(e["kind"] == "group" and e["group"] < e["questions"][0]["id"] for e in ek):
    err("every-kind", "no group whose first question closed")
oo = feeds["omitted-only.json"]
if oo["entries"] or not oo["ok"] or not oo["omitted"]["questions"]:
    err("omitted-only", "shape")
if not any(q["cut"] for e in feeds["partial.json"]["entries"] for q in e.get("questions", [])):
    err("partial", "no cut question")

cmds = json.loads((ROOT / "tests/fixtures/app-commands/commands.json").read_text())
if set(cmds) != {"source", "feeds", "cases"}:
    err("commands", "top-level keys")
CODES = set(re.findall(r"^\| `(\w+)` \| ", cmd_md[cmd_md.index("| `code` | Meaning"):cmd_md.index("### Showing one question")], re.M)) - {"code"}
VERBS = set(re.findall(r"^\| `([a-z-]+)` \| `--(?:entry|question)", cmd_md, re.M))
CLOSED = set(re.findall(r"`(\w+)`", re.search(r"^\| `closed` .*$", cmd_md, re.M).group(0))) - {"closed", "stale"}
seen_verbs, seen_codes, seen_closed = set(), set(), set()
by_name = {c["name"]: c for c in cmds["cases"]}


def norm(d):
    t = (d.get("text") or "").strip() or None
    notes = {k: v.strip() for k, v in d.get("notes", {}).items() if k in d["choices"] and v.strip()}
    return {"choices": d["choices"], "notes": notes, "text": t} if d["choices"] or t else None


def replay(c, e, argv):
    """What the document says the verb does to row `e`: (token, done, skipped)."""
    verb = argv[2]
    qs = [argv[i + 1] for i, a in enumerate(argv) if a == "--question"]
    ids = [int(x) for x in qs]
    detail = json.loads(argv[argv.index("--detail") + 1]) if "--detail" in argv else None
    e = copy.deepcopy(e)
    if verb in ("ack", "stop-showing"):
        return None, [e.get("id", e.get("question_id"))], []
    if verb == "retry":
        e["retries"] += 1
        return token(e), [e["question_id"]], []
    open_ = {q["id"]: q for q in e["questions"]}
    if verb == "draft":
        d = None if detail is None else norm(detail)
        open_[ids[0]]["draft"] = d
        return token(e), ids, []
    if verb == "dismiss":
        done = ids or list(open_)
    elif detail is not None:
        done, skipped = ids, []
        e["questions"] = [q for q in e["questions"] if q["id"] not in done]
        return (token(e) if e["questions"] else None), done, skipped
    else:
        done = [i for i in ids if open_[i]["draft"]]
        skipped = [i for i in ids if not open_[i]["draft"]]
        e["questions"] = [q for q in e["questions"] if q["id"] not in done]
        return (token(e) if e["questions"] else None), done, skipped
    e["questions"] = [q for q in e["questions"] if q["id"] not in done]
    return (token(e) if e["questions"] else None), done, []


for c in cmds["cases"]:
    w = "case " + c["name"]
    a = c["argv"]
    assert a[:2] == ["herdr-inbox", "app"]
    verb = a[2]
    if verb not in VERBS:
        err(w, "verb")
    seen_verbs.add(verb)
    arg = lambda f: a[a.index(f) + 1] if f in a else None
    out = c["stdout"]
    for field, values in (("host", ("up", "gone", "unproven")), ("needs", (None, "B3"))):
        if c[field] not in values:
            err(w, field)
    if type(c["session"]) is not bool or not (c["server"] is None or type(c["server"]) is str):
        err(w, "server, session")
    if verb == "show":
        seen_verbs.add("show")
        if c["effects"] is not None:
            err(w, "show starts nothing")
        if "--token" in a or "--entry" in a:
            err(w, "show takes no token and no entry")
        if c["exit"] != 2 and "--json" not in a:
            err(w, "a case's stdout is JSON: show needs --json here")
        if c["exit"] == 0:
            obj(out, "_show", w)
            r = out["row"]
            full = c["name"] == "show"
            published = {e["key"]: e for e in feeds[c["world"]]["entries"]}.get(r["key"])
            if full:
                if not published or r["token"] != published["token"] or token(r) == r["token"]:
                    err(w, "show keeps the published token")
                if any(x["cut"] for x in r["questions"]) or len(r["questions"][0]["question"].encode()) <= 16384:
                    err(w, "show cuts nothing")
            elif r["token"] != token(r):
                err(w, "token of a left-out row")
            qid = int(a[a.index("--question") + 1])
            if qid not in [x["id"] for x in r["questions"]]:
                err(w, "show row has the question")
            if c["name"] == "show-left-out" and qid not in feeds[c["world"]]["omitted"]["question_ids"]:
                err(w, "left out")
            continue
    if c["world"] not in feeds:
        err(w, "world")
    key = arg("--entry")
    ent = {e["key"]: e for e in feeds[c["world"]]["entries"]}.get(key)
    grammar = "--json" in a and all(CANON.match(a[i + 1]) for i, x in enumerate(a) if x == "--question") \
        and (verb == "show" or re.fullmatch(r"(group|failed|alert):[1-9][0-9]*|remote:.+", key or "") is not None)
    if c["exit"] == 2:
        if out is not None or c["effects"] is not None:
            err(w, "usage has no stdout and no effects")
        if grammar:
            err(w, "exit 2 on well-formed arguments")
        continue
    if not grammar:
        err(w, "malformed arguments need exit 2")
    if not isinstance(out, dict) or out["verb"] != verb or (key and out["entry"] != key):
        err(w, "echo")
    if out["ok"]:
        obj(out, "_ok", w)
        if c["exit"] != 0:
            err(w, "exit")
        if ent is None or arg("--token") != ent["token"]:
            err(w, "success needs the world's token")
            continue
        if verb not in ent["actions"]:
            err(w, "verb not in actions")
        want = replay(c, ent, a)
        if (out["token"], out["done"], out["skipped"]) != want:
            err(w, f"result is not what the verb does: {want}")
        sends = verb in ("answer", "retry") and bool(out["done"])
        fx = c["effects"]
        if not sends:
            if out["delivery"] is not None or fx is not None:
                err(w, "nothing to deliver, nothing starts")
            continue
        host = ent["asker"]["host"]
        core = host == "herdr-core"
        if core != (c["server"] == "herdr-core"):
            err(w, "a core row's server is herdr-core, and only its")
        before_b3 = core and c["needs"] != "B3"
        no_sender = (before_b3 or host == "unknown" or c["host"] == "gone"
                     or (c["server"] is None and not c["session"]))
        want_delivery = "held" if no_sender else "queued"
        if out["delivery"] != want_delivery:
            err(w, f"delivery should be {want_delivery}")
        if before_b3:
            if fx is not None:
                err(w, "before B3 nothing starts for a core row")
            continue
        if fx is None:
            err(w, "the worker is always ensured")
            continue
        if fx["ensure_worker"] != c["server"]:
            err(w, "the stored server must reach the worker")
        if no_sender:
            if fx["sender"] is not None or fx["sender_socket"] is not None:
                err(w, "held starts no sender")
            continue
        if fx["sender_socket"] != (None if core else c["server"]):
            err(w, "the stored server must reach the sender; core sets no socket")
        force = verb == "retry" and host == "herdr-tui"
        if fx["sender"] != ["deliver", *map(str, out["done"])] + (["--force"] if force else []):
            err(w, "sender argv / force")
    else:
        obj(out, "_no", w)
        if c["exit"] != 1 or c["effects"] is not None:
            err(w, "a refusal exits 1 and starts nothing")
        if out["code"] not in CODES:
            err(w, "code")
        seen_codes.add(out["code"])
        if out["closed"] is not None:
            seen_closed.add(out["closed"])
            if out["closed"] not in CLOSED or c["name"] not in ("stale-" + out["closed"], "show-closed") or out["code"] != "stale":
                err(w, "closed word")
        if c["name"] == "stale-token" and (ent is None or arg("--token") == ent["token"] or
                                           arg("--token") != {e["key"]: e for e in ek}[key]["token"]):
            err(w, "must carry every-kind's token, not the world's")
    if c["replay_of"]:
        first = by_name[c["replay_of"]]
        if first["argv"] != a or not first["stdout"]["ok"] or out["code"] != "stale" \
                or first["stdout"]["token"] == arg("--token"):
            err(w, "a replay repeats a done verb, whose token moved, and is stale")
    if isinstance(out, dict) and "--detail" in a and not out["ok"]:
        d = json.loads(arg("--detail"))
        if any(x and x in out["message"] for x in [d.get("text")]):
            err(w, "message echoes the detail text")
    if "--detail" in a and len(arg("--detail").encode()) > 65536:
        err(w, "detail size")
if seen_verbs != VERBS:
    err("cases", f"verbs missing {VERBS-seen_verbs}")
if seen_codes != CODES:
    err("cases", f"codes missing {CODES-seen_codes}")
if seen_closed != CLOSED:
    err("cases", f"closed words missing {CLOSED-seen_closed}")
if by_name["retry-replay"]["replay_of"] != "retry-tui":
    err("cases", "no replay")
for n in ("stop-showing-during-claim", "answer-unproven", "retry-tui", "retry-core"):
    if not by_name[n]["stdout"]["ok"]:
        err(n, "must succeed")

print(f"feeds {len(feeds)}, entries {sum(len(f['entries']) for f in feeds.values())}, kinds {len(kinds)}, "
      f"cases {len(cmds['cases'])}, verbs {len(seen_verbs)}, codes {len(seen_codes)}, closed {len(seen_closed)}, "
      f"errors {len(errors)}")
print("\n".join(errors))
sys.exit(1 if errors else 0)
