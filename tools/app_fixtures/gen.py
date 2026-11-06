#!/usr/bin/env python3
"""Generates app-feed/*.json and app-commands/commands.json under the folder it is given, fake data only.
The fixtures are its output: change them here. usage: gen.py tests/fixtures"""
import json, hashlib, sys
from pathlib import Path

ROOT = Path(sys.argv[1])
T = "2026-01-02T10:%02d:00+00:00"
TITLE, TEXT, PROJECT = "Question Title 123", "Question Text 345", "Project Alpha"
ANSWER, CWD = "Answer Text 567", "/work/project-alpha"
ALPHA, BETA, CORE = "/run/herdr-alpha.sock", "/run/herdr-beta.sock", "herdr-core"
ASKER_F = ("host", "pane", "terminal", "tab_label", "cwd")
Q_F = ("id", "header", "question", "options", "multi_select", "context", "asked_by", "draft")
OPT_F = ("label", "description", "preview")
DET_F = ("choices", "notes", "text")
FIELDS = {"group": ("key", "group", "title", "created_at", "asker", "questions"),
          "failed": ("key", "question_id", "group", "header", "question", "asked_by", "answer", "answered_at",
                     "retries", "asker"),
          "alert": ("key", "id", "alert_kind", "severity", "title", "message", "asked_by", "created_at", "asker")}


def pick(o, fields):
    return {k: o[k] for k in fields}


def token_input(e):
    x = pick(e, FIELDS[e["kind"]])
    x["asker"] = pick(x["asker"], ASKER_F)
    if "questions" in x:
        qs = []
        for q in x["questions"]:
            y = pick(q, Q_F)
            y["options"] = [pick(o, OPT_F) for o in y["options"]]
            y["draft"] = y["draft"] and pick(y["draft"], DET_F)
            qs.append(y)
        x["questions"] = qs
    return x


def token(e):
    c = json.dumps(token_input(e), sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(c.encode()).hexdigest()[:32]


def tui(pane="w1:p1"):
    return {"host": "herdr-tui", "pane": pane, "terminal": None, "tab_label": PROJECT, "cwd": CWD}


def core(terminal="t3"):
    return {"host": "herdr-core", "pane": None, "terminal": terminal, "tab_label": None, "cwd": CWD}


UNKNOWN = {"host": "unknown", "pane": None, "terminal": None, "tab_label": None, "cwd": None}


def opt(label, description=None, preview=None):
    return {"label": label, "description": description, "preview": preview}


def q(id, header=None, options=(), multi=False, context=None, draft=None, asked_by="lane", text=TEXT, cut=False):
    return {"id": id, "header": header, "question": text, "options": list(options), "multi_select": multi,
            "context": context, "asked_by": asked_by, "draft": draft, "cut": cut}


def seal(e, actions):
    e["actions"] = actions
    e["token"] = token(e) if actions else None
    return e


def group(g, minute, asker, questions, title=None):
    e = {"key": f"group:{g}", "kind": "group", "group": g, "title": title, "created_at": T % minute,
         "asker": asker, "questions": questions}
    return seal(e, ["answer", "draft", "dismiss"])


def failed(qid, minute, asker, delivery, retries=0):
    e = {"key": f"failed:{qid}", "kind": "failed", "question_id": qid, "group": qid, "header": None,
         "question": TEXT, "asked_by": "lane", "answer": ANSWER, "answered_at": T % minute, "retries": retries,
         "cut": False, "asker": asker, "delivery": delivery}
    return seal(e, ["retry", "stop-showing"])


def delivery(state, reason=None, wait=None, attempts=0, since=None, auto=True):
    return {"state": state, "reason": reason, "wait": wait, "attempts": attempts, "pending_since": since,
            "auto_retry": auto}


def pending(reason, wait, since=None):
    return delivery("pending", reason, wait, since=since)


def alert(id, kind, severity, minute, asker, message=TEXT):
    e = {"key": f"alert:{id}", "kind": "alert", "id": id, "alert_kind": kind, "severity": severity,
         "title": TITLE, "message": message, "asked_by": "lane", "created_at": T % minute,
         "asker": {**asker, "cwd": None}}  # alerts store no cwd
    return seal(e, ["ack"])


MACHINE = {"id": "machine-alpha", "label": "Machine Alpha", "offline": None}
OFFLINE = {"id": "machine-beta", "label": None, "offline": "not reached yet"}


def remote_alert():
    return seal({"key": "ralert:machine-alpha|2", "kind": "remote_alert", "machine": MACHINE, "id": 2,
                 "alert_kind": "agent", "severity": "info", "title": TITLE, "message": None,
                 "tab_label": PROJECT, "asked_by": "lane", "created_at": T % 3}, [])


def remote_group():
    return seal({"key": "remote:machine-alpha|db-alpha|4", "kind": "remote_group", "machine": MACHINE,
                 "title": TITLE, "created_at": T % 4, "tab_label": PROJECT,
                 "questions": [q(4, options=[opt("Option A"), opt("Option B")]), q(5)]}, [])


def outbox(request, machine, state, result=None, action="answer", rid=7):
    return seal({"key": f"routbox:{request}", "kind": "remote_outbox", "machine": machine, "action": action,
                 "remote_id": rid, "state": state, "result": result, "sending": False, "expired": False,
                 "expires_at": T % 59}, [])


def problem():
    return seal({"key": "rproblem:machine-alpha|9", "kind": "remote_problem", "machine": MACHINE, "remote_id": 9,
                 "why": "agent closed — answer kept for check_answers", "question": TEXT, "answer": ANSWER,
                 "cut": False}, [])


G12_OPTIONS = [opt("Option A", "Option Description 1", "Option Preview 1"), opt("Option B")]
DRAFT14 = {"choices": ["Option A", "Option C"], "notes": {"Option A": "Note Text 1"}, "text": None}
ZERO = {"groups": 0, "questions": 0, "failed": 0, "alerts": 0, "remote": 0, "question_ids": []}


def g14_questions():
    return [q(14, "Header 1", [opt("Option A"), opt("Option B"), opt("Option C")], True, "Context Text 678", DRAFT14),
            q(15, "Header 2", draft={"choices": [], "notes": {}, "text": ANSWER}),
            q(16, "Header 3", [opt("Option A"), opt("Option B")])]


def feed(entries, ok=True, errors=(), omitted=ZERO, new=(41, 6), at=1767348000):
    return {"v": 1, "generated_at": at, "ok": ok, "errors": list(errors), "omitted": omitted,
            "new_since": {"question_id": new[0], "alert_id": new[1]}, "entries": entries}


GONE = delivery("agent_gone", "no live agent in pane w1:p3", None, 10, auto=False)


def every_kind():
    return feed([
        alert(5, "agent", "urgent", 1, tui()),
        alert(6, "blocked", "warning", 2, tui("w1:p2")),
        alert(4, "agent", "info", 0, core("t6")),
        remote_alert(),
        group(12, 10, tui(), [q(12, "Header 1", G12_OPTIONS)]),
        group(14, 12, core(), g14_questions(), TITLE),
        group(17, 14, UNKNOWN, [q(17, asked_by=None)]),
        group(18, 15, tui("w1:p4"), [q(19, "Header 2")], TITLE),  # Q18 is closed: the group keeps its id
        failed(7, 20, tui(), delivery("claimed", attempts=1)),
        failed(8, 21, tui(), pending("busy: agent is working", "busy", T % 22)),
        failed(9, 22, core("t4"), delivery("failed", "no live agent in pane", None, 3, auto=False)),
        failed(10, 23, tui("w1:p3"), GONE),
        failed(11, 24, core("t5"), GONE),
        failed(13, 25, tui(), delivery("unattempted")),
        failed(30, 26, tui(), pending("busy: typed while the agent was busy but never seen; waiting for it to be idle",
                                      "busy_idle", T % 27)),
        failed(31, 27, tui(), pending("requeued: not delivered earlier", "requeued")),
        failed(32, 28, tui(), pending("blocked: panel: the agent's background-task panel has focus", "panel")),
        failed(33, 29, tui(), pending("blocked: agent is at a prompt", "blocked")),
        failed(34, 30, tui(), pending("waiting: the operator is in that pane", "operator_here")),
        failed(35, 31, tui(), pending("unsent text in the input box", "other")),
        failed(37, 32, UNKNOWN, delivery("failed", "no pane", None, 1, auto=False), retries=1),
        remote_group(),
        outbox("request-alpha", MACHINE, "queued"),
        outbox("request-beta", OFFLINE, "rejected", TEXT, "dismiss", 8),
        problem(),
    ])


def stale_token():
    """every-kind after the agent withdrew Q16 from group 14."""
    f = every_kind()
    i = next(n for n, e in enumerate(f["entries"]) if e["key"] == "group:14")
    f["entries"][i] = group(14, 12, core(), g14_questions()[:2], TITLE)
    f["generated_at"] += 5
    return f


LONG = "Question Text 345 " * 1000  # 18000 bytes


def show_row():
    """Q14 of partial.json uncut, with the token of its cut form."""
    e = group(14, 12, core(), [q(14, text=LONG)])
    e["token"] = group(14, 12, core(), [q(14, text=LONG[:16384], cut=True)])["token"]
    return e


def partial():
    long_text = LONG[:16384]
    return feed([alert(5, "agent", "urgent", 1, tui()),
                 group(12, 10, tui(), [q(12, "Header 1", G12_OPTIONS)]),
                 group(14, 12, core(), [q(14, text=long_text, cut=True)])],
                errors=["remote machines could not be read", "3 entries left out to stay under 1 MiB"],
                omitted={"groups": 1, "questions": 2, "failed": 1, "alerts": 0, "remote": 1, "question_ids": [21, 22]})


def unknown_vocabulary():
    """A newer writer: every word a v1 reader must survive, and a field it must ignore."""
    g = group(12, 10, {**UNKNOWN, "host": "future-host"}, [q(12)])
    g["future_field"] = "Question Text 345"
    g["questions"][0]["future_field"] = 1
    f = failed(8, 21, tui(), delivery("future_state", "Question Text 345", "future_wait", 0))
    a = alert(5, "future_kind", "future_severity", 1, tui())
    return feed([a, g, f, {"key": "future:1", "kind": "future_kind", "token": None, "actions": [],
                           "future_field": TITLE}])


def write(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=1, ensure_ascii=False) + "\n")


def entry(f, key):
    return next(e for e in f["entries"] if e["key"] == key)


def ok(verb, key, token=None, done=(), skipped=(), delivery=None):
    return {"v": 1, "ok": True, "verb": verb, "entry": key, "token": token, "done": list(done),
            "skipped": list(skipped), "delivery": delivery}


def no(verb, key, code, message, closed=None):
    return {"v": 1, "ok": False, "verb": verb, "entry": key, "code": code, "message": message, "closed": closed}


def commands():
    f = every_kind()
    t = lambda key: entry(f, key)["token"]
    g14 = lambda qs: group(14, 12, core(), qs, TITLE)["token"]

    def bumped(qid):
        e = dict(entry(f, f"failed:{qid}"))
        e["retries"] += 1
        return token(e)

    d = lambda *choices, text=None: json.dumps({"choices": list(choices), "notes": {}, "text": text})
    qs = g14_questions()
    qs[2]["draft"] = {"choices": ["Option A"], "notes": {}, "text": None}
    drafted = g14(qs)
    qs = g14_questions()
    qs[0]["draft"] = None
    cleared = g14(qs)
    left = g14(g14_questions()[2:])
    qs = g14_questions()
    dismissed = g14([qs[0], qs[2]])
    changed = "This group changed; look again."
    E = "every-kind.json"
    app = ["herdr-inbox", "app"]
    cases = []

    def C(name, about, argv, exit, stdout, world=E, server=ALPHA, host="up", session=True, needs=None, effects=None,
          replay_of=None):
        cases.append({"name": name, "about": about, "world": world, "server": server, "host": host,
                      "session": session, "needs": needs, "argv": app + argv, "exit": exit, "stdout": stdout,
                      "effects": effects, "replay_of": replay_of})

    def fx(server, *sender):
        socket = None if server == CORE else server
        return {"ensure_worker": server, "sender": ["deliver", *sender] if sender else None,
                "sender_socket": socket if sender else None}

    def verb(v, key, *rest):
        return [v, "--entry", key, "--token", t(key) if key in tokens else "0" * 32, *rest, "--json"]

    tokens = {e["key"] for e in f["entries"]}
    C("answer-lone", "A lone open question, answered at once.",
      verb("answer", "group:12", "--question", "12", "--detail", d("Option A")), 0,
      ok("answer", "group:12", done=[12], delivery="queued"), effects=fx(ALPHA, "12"))
    C("answer-unproven", "Setup: the server timed out the probe. That proves nothing: the sender goes.",
      verb("answer", "group:12", "--question", "12", "--detail", d("Option A")), 0,
      ok("answer", "group:12", done=[12], delivery="queued"), host="unproven", effects=fx(ALPHA, "12"))
    C("answer-submit", "The Submit tab on a core row: two of three questions, the group stays.",
      verb("answer", "group:14", "--question", "14", "--question", "15"), 0,
      ok("answer", "group:14", left, [14, 15], delivery="queued"), server=CORE, needs="B3",
      effects=fx(CORE, "14", "15"))
    C("answer-held-core-before-b3", "A core row before B3: recorded, no worker, no sender. check_answers has it.",
      verb("answer", "group:14", "--question", "14", "--question", "15"), 0,
      ok("answer", "group:14", left, [14, 15], delivery="held"), server=CORE, host="gone")
    C("answer-held-core-down", "Setup: hc proves herdr-core is not running. The core worker is ensured, no sender.",
      verb("answer", "group:14", "--question", "14", "--question", "15"), 0,
      ok("answer", "group:14", left, [14, 15], delivery="held"), server=CORE, host="gone", needs="B3",
      effects=fx(CORE))
    C("answer-skips-no-draft", "A named question with no draft is skipped, as Submit does.",
      verb("answer", "group:14", "--question", "14", "--question", "15", "--question", "16"), 0,
      ok("answer", "group:14", left, [14, 15], [16], "queued"), server=CORE, needs="B3", effects=fx(CORE, "14", "15"))
    C("answer-nothing-to-send", "Only a question with no draft: nothing changes, nothing starts.",
      verb("answer", "group:14", "--question", "16"), 0,
      ok("answer", "group:14", t("group:14"), [], [16]), server=CORE)
    C("answer-first-question-closed", "A group whose first question closed earlier keeps its id.",
      verb("answer", "group:18", "--question", "19", "--detail", d(text=ANSWER)), 0,
      ok("answer", "group:18", done=[19], delivery="queued"), effects=fx(ALPHA, "19"))
    C("answer-held-no-server", "Setup: herdr says NotRunning on the stored socket. Recorded, the worker ensured, no sender.",
      verb("answer", "group:12", "--question", "12", "--detail", d("Option A")), 0,
      ok("answer", "group:12", done=[12], delivery="held"), host="gone", effects=fx(ALPHA))
    C("answer-held-no-pane", "A row with neither a pane nor a terminal: only check_answers can return it.",
      verb("answer", "group:17", "--question", "17", "--detail", d(text=ANSWER)), 0,
      ok("answer", "group:17", done=[17], delivery="held"), server=None, effects=fx(None))
    C("draft", "A draft changes the token: the result has the new one.",
      verb("draft", "group:14", "--question", "16", "--detail", d("Option A")), 0,
      ok("draft", "group:14", drafted, [16]))
    C("draft-clear", "Clearing a draft changes the token too.",
      verb("draft", "group:14", "--question", "14", "--clear"), 0, ok("draft", "group:14", cleared, [14]))
    C("draft-normalises-to-nothing", "A detail that normalises to nothing clears the draft.",
      verb("draft", "group:14", "--question", "14", "--detail", d(text="   ")), 0,
      ok("draft", "group:14", cleared, [14]))
    C("dismiss-question", "One question of a group.",
      verb("dismiss", "group:14", "--question", "15"), 0, ok("dismiss", "group:14", dismissed, [15]))
    C("dismiss-group", "The whole group: the row is gone.",
      verb("dismiss", "group:14"), 0, ok("dismiss", "group:14", done=[14, 15, 16]))
    C("retry-tui", "A TUI row is forced. The row stays, with a new token.",
      verb("retry", "failed:8"), 0, ok("retry", "failed:8", bumped(8), [8], delivery="queued"),
      effects=fx(ALPHA, "8", "--force"))
    C("retry-core", "A core row is never forced.",
      verb("retry", "failed:9"), 0, ok("retry", "failed:9", bumped(9), [9], delivery="queued"),
      server=CORE, needs="B3", effects=fx(CORE, "9"))
    C("retry-held-core-before-b3", "A core row before B3: the counter moves, nothing starts.",
      verb("retry", "failed:9"), 0, ok("retry", "failed:9", bumped(9), [9], delivery="held"), server=CORE, host="gone")
    C("retry-other-server", "The stored server goes to both the worker and the sender, not the default socket.",
      verb("retry", "failed:13"), 0, ok("retry", "failed:13", bumped(13), [13], delivery="queued"),
      server=BETA, effects=fx(BETA, "13", "--force"))
    C("retry-no-stored-server", "No stored server but a session: the verb's own default, under the conversation guard.",
      verb("retry", "failed:7"), 0, ok("retry", "failed:7", bumped(7), [7], delivery="queued"),
      server=None, effects=fx(None, "7", "--force"))
    C("retry-held-no-session", "No stored server and no session: the recipient rule beats retry.",
      verb("retry", "failed:33"), 0, ok("retry", "failed:33", bumped(33), [33], delivery="held"),
      server=None, session=False, effects=fx(None))
    C("retry-held", "Setup: herdr says NotRunning on the stored socket. The counter moves, the worker is ensured, no sender.",
      verb("retry", "failed:10"), 0, ok("retry", "failed:10", bumped(10), [10], delivery="held"),
      host="gone", effects=fx(ALPHA))
    C("retry-replay", "Setup: retry-tui ran and its result was lost. The same command again is refused.",
      verb("retry", "failed:8"), 1, no("retry", "failed:8", "stale", changed), replay_of="retry-tui")
    C("retry-already-sending", "Setup: another process took a live claim on Q8 after this feed was read.",
      verb("retry", "failed:8"), 1, no("retry", "failed:8", "already_sending", "Q8 is already being delivered"))
    C("stop-showing", "The ⚠ row goes; check_answers still has the answer.",
      verb("stop-showing", "failed:10"), 0, ok("stop-showing", "failed:10", done=[10]))
    C("stop-showing-during-claim", "Setup: a process holds a live claim on Q30. The row is hidden, the send goes on.",
      verb("stop-showing", "failed:30"), 0, ok("stop-showing", "failed:30", done=[30]))
    C("ack", "An alert acknowledged.", verb("ack", "alert:5"), 0, ok("ack", "alert:5", done=[5]))
    C("stale-token", "The feed was read before the agent withdrew Q16: the database is stale-token.json.",
      verb("answer", "group:14", "--question", "14", "--question", "15"), 1,
      no("answer", "group:14", "stale", changed), world="stale-token.json")
    bad = ["ack", "--entry", "alert:5", "--token", "0" * 32, "--json"]
    C("stale-alert-token", "A token that does not match, on an open alert.", bad, 1,
      no("ack", "alert:5", "stale", changed))
    closures = [("answered", "group:12", ("answer", "--question", "12", "--detail", d("Option A")), "the TUI popup answered Q12"),
                ("dismissed", "group:12", ("dismiss",), "the TUI popup dismissed Q12"),
                ("withdrawn", "group:12", ("dismiss",), "the agent withdrew Q12"),
                ("removed", "group:12", ("dismiss",), "Q12 is no longer in the database"),
                ("delivered", "failed:8", ("retry",), "Q8 was delivered"),
                ("fetched", "failed:8", ("retry",), "the agent fetched Q8 with check_answers"),
                ("hidden", "failed:8", ("retry",), "the popup stopped showing Q8"),
                ("acked", "alert:6", ("ack",), "the popup acknowledged A6"),
                ("cleared", "alert:6", ("ack",), "the agent is no longer waiting")]
    for word, key, rest, why in closures:
        C(f"stale-{word}", f"Setup: {why} after this feed was read.", verb(rest[0], key, *rest[1:]), 1,
          no(rest[0], key, "stale", changed, word))
    C("invalid-choice", "A label the question does not have.",
      verb("answer", "group:12", "--question", "12", "--detail", d("Option Z")), 1,
      no("answer", "group:12", "invalid", "Q12 has no such option: Option Z"))
    C("invalid-two-choices", "Two choices on a question without multi_select.",
      verb("answer", "group:12", "--question", "12", "--detail", d("Option A", "Option B")), 1,
      no("answer", "group:12", "invalid", "Q12 takes one choice"))
    C("invalid-repeated-choice", "One label twice.",
      verb("draft", "group:14", "--question", "14", "--detail", d("Option A", "Option A")), 1,
      no("draft", "group:14", "invalid", "Q14 has a choice twice"))
    C("invalid-empty-answer", "A detail that normalises to nothing is no answer.",
      verb("answer", "group:12", "--question", "12", "--detail", d(text="   ")), 1,
      no("answer", "group:12", "invalid", "Q12 has no answer"))
    C("invalid-other-group", "A --question of another group.",
      verb("draft", "group:14", "--question", "12", "--clear"), 1,
      no("draft", "group:14", "invalid", "Q12 is not an open question of this group"))
    C("invalid-detail-on-several-open", "--detail with answer needs a lone open question.",
      verb("answer", "group:14", "--question", "14", "--detail", d("Option A")), 1,
      no("answer", "group:14", "invalid", "This group has several open questions: use draft, then answer"))
    remote = "remote:machine-alpha|db-alpha|4"
    C("read-only-remote", "Remote rows take no verbs in v1; they have no token.",
      verb("answer", remote, "--question", "4", "--detail", d("Option A")), 1,
      no("answer", remote, "read_only", "Answer this in the herdr popup"))
    C("read-only-wrong-verb", "A verb the row's actions do not list.",
      verb("ack", "failed:8"), 1, no("ack", "failed:8", "read_only", "This row does not take that action"))
    C("busy", "Setup: another process holds the write lock for the whole budget.", verb("ack", "alert:5"), 1,
      no("ack", "alert:5", "busy", "The inbox database is busy; try again in a moment"))
    C("not-migrated", "Setup: the database is not at this code's revision.", verb("ack", "alert:5"), 1,
      no("ack", "alert:5", "not_migrated", "The inbox is being upgraded; try again in a few seconds"),
      world="not-migrated.json")
    C("error", "Setup: the database file cannot be read.", verb("ack", "alert:5"), 1,
      no("ack", "alert:5", "error", "The inbox database failed: unable to open database file"))
    C("show", "A cut question in full: the published token, no text cut.",
      ["show", "--question", "14", "--json"], 0,
      {"v": 1, "ok": True, "verb": "show", "entry": "group:14", "row": show_row()}, world="partial.json")
    C("show-left-out", "Q21 is in omitted.question_ids. The row and its token let the overlay answer it.",
      ["show", "--question", "21", "--json"], 0,
      {"v": 1, "ok": True, "verb": "show", "entry": "group:21",
       "row": group(21, 16, tui(), [q(21), q(22)], TITLE)}, world="omitted-only.json")
    C("show-closed", "Setup: Q12 was answered. show only reads open questions.",
      ["show", "--question", "12", "--json"], 1, no("show", "group:12", "stale", changed, "answered"))
    C("usage-no-json", "No --json: usage on stderr, nothing on stdout, no I/O.",
      ["ack", "--entry", "alert:5", "--token", t("alert:5")], 2, None)
    C("usage-bad-entry", "A malformed --entry needs no database to refuse.",
      ["ack", "--entry", "alert:05", "--token", t("alert:5"), "--json"], 2, None)
    C("usage-bad-id", "A non-canonical id.",
      ["draft", "--entry", "group:14", "--token", t("group:14"), "--question", "Q14", "--clear", "--json"], 2, None)
    return {"source": "tools/app_fixtures/gen.py, for docs/protocol/app-commands-v1.md",
            "feeds": "tests/fixtures/app-feed/", "cases": cases}


feeds = ROOT / "app-feed"
write(feeds / "every-kind.json", every_kind())
write(feeds / "stale-token.json", stale_token())
write(feeds / "empty.json", feed([]))
write(feeds / "partial.json", partial())
write(feeds / "omitted-only.json", feed([], errors=["1 entry left out to stay under 1 MiB"],
                                        omitted={"groups": 1, "questions": 2, "failed": 0, "alerts": 0, "remote": 0,
                                                 "question_ids": [21, 22]}))
write(feeds / "not-migrated.json", feed([], ok=False, errors=["the inbox database is not migrated to this version yet"],
                                        new=(0, 0)))
write(feeds / "unknown-vocabulary.json", unknown_vocabulary())
write(ROOT / "app-commands" / "commands.json", commands())
