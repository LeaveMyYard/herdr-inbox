"""Capture the popup's transition golden, ``tests/fixtures/popup-transitions.json``, from
the running Python popup (``tui.InboxApp`` under Textual's ``run_test``). Nothing in it is
written by hand: every ``state`` and ``effects`` entry is read off the Python app.

The golden is the popup's key-safety contract written down as data: scenarios of
``(state, event) -> (state, effects)``. The Rust popup model (``inbox-core::popup``,
``crates/inbox-core/tests/popup_transitions.rs``) replays it, and so does the herdr-app
mod's JavaScript model (plan herdr-app-inbox §2.3, B6). The format is described in
``tests/fixtures/README.md`` ("popup-transitions.json").

How it is captured: the popup runs with the REAL delays (ARM_SECONDS 0.4, ROW_ARM_SECONDS 1,
GRACE_SECONDS 0.5, RELEASE_GRACE 0.5, ALERT_GO_GRACE 0.5) on a held clock that only the
scenario moves (integer milliseconds), so every arming window is exact and no runner speed
changes a byte. Polls happen only when a scenario says so, with the database as the scenario
left it. The popup's IO is cut at the model's boundary and recorded as effects: drafts,
sends (recorded, then completed by an explicit ``sent`` event, as a detached send reports
back), dismissals, hides, acknowledgements, go-to-agent requests (completed by ``focused``),
notifications and exits. Deterministic: TZ=UTC, every ``HERDR_*`` variable dropped, a temporary
inbox database per scenario, the tests' fake herdr; nothing of the operator's is read.

    uv run python tools/capture_popup_transitions.py              # rewrite the golden
    uv run python tools/capture_popup_transitions.py --out FILE   # write FILE instead
"""

from __future__ import annotations

import argparse
import asyncio
import json
import shutil
import sys
import tempfile
import types
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
GOLDEN = ROOT / "tests" / "fixtures" / "popup-transitions.json"
START_MS = 10_000  # the held clock's first reading (ms); Python's own defaults (0) are "long ago"
HUMAN_GAP_MS = 600  # a deliberate key: past every arming and grace window
SCHEMA = 1


class Clock:
    def __init__(self) -> None:
        self.ms = START_MS

    def seconds(self) -> float:
        return self.ms / 1000.0


def ms(seconds: float | None) -> int | None:
    """Seconds on the held clock as integer ms; Python's 0 default ("never") is null."""
    return None if not seconds else round(seconds * 1000)


def entry_id(entry) -> str | None:
    """An entry as the golden names it: ``kind:key`` (``gone`` alone for the summary row)."""
    if entry is None:
        return None
    kind, key = entry
    return "gone" if kind == "gone" else f"{kind}:{key}"


def tab_id(tab) -> str | None:
    if tab is None:
        return None
    return "submit" if tab[0] == "submit" else f"q:{tab[1]}"


# -- the snapshot: what the IO layer hands the model at open and at every poll ---------------

def question_json(q) -> dict:
    """What the model decides with (the IO layer's Question carries more, for the renderer)."""
    return {"id": q.id, "group": q.group, "status": q.status, "pane_id": q.pane_id,
            "options": [o["label"] for o in (q.options or [])], "multi_select": bool(q.multi_select),
            "has_preview": any(o.get("preview") for o in (q.options or [])),
            "draft": q.draft_json, "delivery_error": q.delivery_error, "delivered": q.delivered_at is not None}


def alert_json(a) -> dict:
    return {"id": a.id, "kind": a.kind, "status": a.status, "pane_id": a.pane_id}


def snapshot(store) -> dict:
    """The database as the popup reads it (the same queries): every question and alert row
    (the scenarios' databases are small), and the lists the popup builds its entries from."""
    from herdr_inbox import alerts

    qs = store.list(include_closed=True)
    rows = store.db.execute("SELECT * FROM alerts ORDER BY id").fetchall()
    return {
        "questions": [question_json(q) for q in sorted(qs, key=lambda q: q.id)],
        "open": sorted(q.id for q in store.list()),
        "failed": [q.id for q in store.failed_deliveries()],
        "alerts": [alert_json(alerts.Alert(**dict(r))) for r in rows],
        "open_alerts": [a.id for a in alerts.open_alerts(store)],
    }


# -- the state: what the model must agree on after every event --------------------------------

def view_state(app) -> dict:
    from textual.widgets import Input

    from herdr_inbox import tui

    found = app.base.query("#body > *")
    v = found.first() if found else None
    if v is None:
        return {"kind": "none"}
    if isinstance(v, tui.QuestionView):
        boxes = list(v.query("#inline"))
        box = boxes[0] if boxes and isinstance(boxes[0], Input) else None
        kind, opt = v.rows[v.cursor]
        return {
            "kind": "question", "qid": v.q.id, "in_group": v.in_group, "last": v.last,
            "cursor": kind if opt is None else f"{kind}:{opt}", "picked": sorted(v.picked),
            "notes": {str(k): n for k, n in sorted(v.notes.items())}, "text": v.text,
            "editing": None if not v.editing else (v.editing[0] if v.editing[1] is None else f"note:{v.editing[1]}"),
            "input": None if box is None else {"value": box.value, "cursor": box.cursor_position},
            "armed_at": ms(getattr(v, "armed_at", None)),
        }
    if isinstance(v, tui.ReviewView):
        cur = v.cursor
        return {"kind": "review", "items": [q.id for q in v.items],
                "cursor": f"item:{cur[1]}" if isinstance(cur, tuple) else cur,
                "armed_at": ms(getattr(v, "armed_at", None))}
    if isinstance(v, tui.GoneView):
        return {"kind": "gone", "ref": v.ref, "reason": v.reason}
    if isinstance(v, tui.FailedView):
        return {"kind": "failed", "qid": v.q.id, "cursor": v.cursor, "armed_at": ms(getattr(v, "armed_at", None))}
    if isinstance(v, tui.AlertView):
        return {"kind": "alert", "alert": v.alert.id, "label": v.label, "armed_at": ms(getattr(v, "armed_at", None))}
    if isinstance(v, tui.GroupListView):
        return {"kind": "list", "cursor": entry_id(v.cursor), "armed_at": ms(getattr(v, "armed_at", None))}
    raise AssertionError(f"unknown view {type(v).__name__}")


def focus_name(app) -> str:
    """Which widget has the keys: the view on screen, its text box, the tab row (#chips), the
    view's scrolling option list (.choices), or nothing."""
    from textual.containers import HorizontalScroll, VerticalScroll
    from textual.widgets import Input

    f = app.screen.focused if len(app.screen_stack) == 1 else app.screen_stack[0].focused
    if f is None:
        return "none"
    if isinstance(f, Input):
        return "box"
    if isinstance(f, HorizontalScroll):
        return "chips"
    if isinstance(f, VerticalScroll):
        return "preview" if f.has_class("preview-box") else "choices"
    found = app.base.query("#body > *")
    if found and f is found.first():
        return "view"
    return type(f).__name__


def state(app, rec) -> dict:
    from herdr_inbox import tui

    if rec.exited:
        return {"running": False}
    confirm = None
    if len(app.screen_stack) > 1 and isinstance(app.screen, tui.ConfirmDismiss):
        confirm = app.screen.what
    return {
        "running": True,
        "focus": focus_name(app),
        "mode": app.mode,
        "entries": [entry_id(e) for e in app.entries],
        "closed": {entry_id(e): r for e, r in app.closed.items()},
        "fresh_until": {entry_id(e): ms(t + tui.ROW_ARM_SECONDS) for e, t in app.arrived_at.items()
                        if e in app.entries},
        "list_cursor": entry_id(app.list_cursor),
        "direct": app.direct,
        "held": app.held,
        "entry": entry_id(app.entry),
        "tabs": [tab_id(t) for t in app.tabs],
        "current": tab_id(app.current),
        "gone": {tab_id(t): r for t, r in app.gone.items()},
        "single": app.single,
        "grace_until": ms(app.grace_until),
        "view_at": ms(getattr(app, "view_at", None)),
        "last_key_at": ms(getattr(app, "last_key_at", None)),
        "prev_key_at": ms(getattr(app, "prev_key_at", None)),
        "view": view_state(app),
        "confirm": confirm,
    }


# -- the recorder: the popup's IO cut at the model's boundary ---------------------------------

class Recorder:
    """Patches the popup's IO so every call becomes an effect (and the database writes the
    model's effects stand for still happen, so the popup reads what it would)."""

    def __init__(self) -> None:
        self.on = False
        self.effects: list[dict] = []
        self.sends: list[tuple[list[int], int]] = []  # (done, wanted) waiting for their `sent` event
        self.exited = False
        self._inner = 0

    def add(self, effect: dict) -> None:
        if self.on and not self._inner:
            self.effects.append(effect)

    def take(self) -> list[dict]:
        out, self.effects = self.effects, []
        return out

    def install(self, clock: Clock) -> None:
        from herdr_inbox import alerts, deeplink, service, tui
        from herdr_inbox.store import Store

        rec = self
        tui.time = types.SimpleNamespace(monotonic=clock.seconds)  # the popup's clock only
        tui._go_clock = clock.seconds
        tui.POLL_SECONDS = 10**6  # polls are scenario events
        tui.ARM_SECONDS, tui.ROW_ARM_SECONDS, tui.GRACE_SECONDS = 0.4, 1.0, 0.5
        tui.RELEASE_GRACE, tui.ALERT_GO_GRACE = 0.5, 0.5
        tui.LIVE_TAB_LABELS, tui.ENABLE_BACKGROUND = False, False
        deeplink.hold_popup_lock = lambda: None
        tui.InboxApp.check_radar = lambda self: None

        real_set_draft = Store.set_draft

        def set_draft(self, qid, detail):
            rec.add({"draft": {"qid": qid, "detail": detail}})
            return real_set_draft(self, qid, detail)

        Store.set_draft = set_draft

        real_close = service.close

        def close(store, qid, status, note=None):
            if status == "dismissed":
                rec.add({"dismiss": {"qid": qid}})
            return real_close(store, qid, status, note)

        service.close = close

        real_hide = Store.hide_failed

        def hide_failed(self, qid):
            rec.add({"hide": {"qid": qid}})
            return real_hide(self, qid)

        Store.hide_failed = hide_failed

        real_ack = alerts.acknowledge

        def acknowledge(store, aid):
            rec.add({"ack": {"alert": aid}})
            return real_ack(store, aid)

        alerts.acknowledge = acknowledge

        def notify(self, message, *, title="", severity="information", timeout=None, markup=True):
            rec.add({"notify": {"text": str(message), "severity": severity}})

        tui.InboxApp.notify = notify

        def exit_(self, result=None, return_code=0, message=None):
            rec.add({"exit": {}})
            rec.exited = True

        tui.InboxApp.exit = exit_

        async def action_quit(self):
            rec.add({"quit": {}})  # waits for an in-flight send first (P6-D), then exits
            rec.exited = True

        tui.InboxApp.action_quit = action_quit

        real_answer, real_submit = service.answer, service.submit

        def run_worker(self, work, *args, group="default", **kwargs):
            cells = dict(zip(work.__code__.co_freevars, (c.cell_contents for c in work.__closure__ or ())))
            if group == "focus":
                rec.add({"go_to": {"pane": cells["pane"], "graced": cells["graced"]}})
                return None
            if group != "send":
                raise AssertionError(f"unexpected worker {group}")
            seen: list[dict] = []

            inner: list[int] = []

            def answer(s, qid, text=None, *, choices=None, notes=None, deliver=True):
                seen.append({"send": {"kind": "answer", "qid": qid,
                                      "detail": service.make_detail(text, choices, notes)}})
                inner.append(1)
                try:
                    return real_answer(s, qid, text, choices=choices, notes=notes, deliver=False)
                finally:
                    inner.pop()

            def submit(s, qids, deliver=True):
                if not inner:  # service.answer submits too: one effect
                    seen.append({"send": {"kind": "submit", "qids": list(qids)}})
                return real_submit(s, qids, deliver=False)

            service.answer, service.submit = answer, submit
            rec._inner += 1
            try:
                with Store(cells["store"].path) as own:
                    done = cells["work"](own, False)
            finally:
                rec._inner -= 1
                service.answer, service.submit = real_answer, real_submit
            qids = cells["qids"]
            if not seen:  # the ⚠ entry's Retry / Send now (it records nothing itself)
                seen.append({"send": {"kind": "retry", "qid": qids[0]}})
            for effect in seen:
                rec.add(effect)
            rec.sends.append(([q.id for q in done], len(qids)))
            return None

        tui.InboxApp.run_worker = run_worker


# -- scenarios -------------------------------------------------------------------------------

class Run:
    """One scenario: a database set up by ``setup``, the popup opened, then steps."""

    def __init__(self, name: str, doc: str, setup, focus: str | None, deeplink: dict | None = None):
        self.name, self.doc, self.setup, self.focus, self.deeplink = name, doc, setup, focus, deeplink


def callers():
    from herdr_inbox.service import Caller

    sid = "aaaaaaaa-0000-4000-8000-00000000000a"
    return (Caller("w1:p1", "w1:t1", "tab-A", sid, "/r"), Caller("w1:p2", "w1:t2", "tab-B", sid, "/r"),
            Caller("w1:p3", "w1:t3", "tab-C", sid, "/r"))


async def run_scenario(scn, steps, work: Path, rec: Recorder, clock: Clock) -> dict:
    import os

    from herdr_inbox import deeplink, tui
    from herdr_inbox.store import Store

    os.environ["HERDR_INBOX_DB"] = str(work / f"{scn.name}.db")
    store = Store(migrate=True)
    clock.ms = START_MS
    rec.on, rec.effects, rec.sends, rec.exited = False, [], [], False
    scn.setup(store)
    request = None
    if scn.deeplink:
        request = {**scn.deeplink["target"], "hold": scn.deeplink.get("hold", True)}  # deeplink.consume_request's
    deeplink.consume_request = lambda: request
    deeplink.resolve = lambda s, r: scn.deeplink["resolved"] if scn.deeplink else None
    opened = {"focus_pane": scn.focus, "snapshot": snapshot(store)}
    if scn.deeplink:
        resolved = scn.deeplink["resolved"]
        opened["deeplink"] = {"target": None if resolved is None else entry_id(resolved[0]),
                              "qid": None if resolved is None else resolved[1],
                              "hold": scn.deeplink.get("hold", True),
                              "missing": f"{deeplink.describe(scn.deeplink['target'])} is no longer open"}
    out = {"name": scn.name, "doc": scn.doc, "steps": []}
    app = tui.InboxApp(store, focus_pane=scn.focus)
    rec.on = True  # the popup's own on_mount effects count (a deep link's notice)
    async with app.run_test(size=(100, 30)) as pilot:
        await settle(pilot)
        out["steps"].append({"at": clock.ms, "event": {"open": opened}, "effects": rec.take(), "state": state(app, rec)})
        for step in steps:
            if rec.exited:
                if os.environ.get("POPUP_CAPTURE_LENIENT"):  # probing: keep what ran
                    break
                raise AssertionError(f"{scn.name}: a step after the popup exited")
            event = await step(app, pilot, store, rec, clock)
            await settle(pilot)
            out["steps"].append({"at": clock.ms, "event": event, "effects": rec.take(), "state": state(app, rec)})
        rec.on = False
    store.close()
    return out


async def settle(pilot) -> None:
    for _ in range(4):
        await pilot.pause()


def key(name: str, gap: int = HUMAN_GAP_MS):
    async def step(app, pilot, store, rec, clock):
        clock.ms += gap
        await pilot.press(name)
        return {"key": name}

    return step


def keys(*names: str, gap: int = HUMAN_GAP_MS):
    return [key(n, gap) for n in names]


def typed(text: str, gap: int = 50):
    return [key("space" if c == " " else c, gap) for c in text]


def unseen(change, gap: int = 100):
    """The database changes and the popup does not poll. Python reads it live at its next
    action; the model gets the read as a ``data`` event (rows change only at polls)."""
    async def step(app, pilot, store, rec, clock):
        clock.ms += gap
        rec.on = False
        change(store)
        rec.on = True
        return {"data": snapshot(store)}

    return step


def poll(change=None, gap: int = 100):
    """Something changes in the database (``change(store)``), then the popup polls."""
    async def step(app, pilot, store, rec, clock):
        clock.ms += gap
        rec.on = False
        if change is not None:
            change(store)
        rec.on = True
        app.poll()
        return {"poll": snapshot(store)}

    return step


def sent(gap: int = 100):
    """The oldest recorded send reports back (a detached send: recorded, delivery handed off)."""
    async def step(app, pilot, store, rec, clock):
        from herdr_inbox import tui

        clock.ms += gap
        done, wanted = rec.sends.pop(0)
        snap = snapshot(store)
        qs = [store.get(i) for i in done]
        tui._after_submit(app, qs, wanted, detached=True)
        return {"sent": {"done": done, "wanted": wanted, "snapshot": snap}}

    return step


def focused(pane: str, error: str | None = None, gap: int = 100):
    async def step(app, pilot, store, rec, clock):
        clock.ms += gap
        app._focused(pane, error)
        return {"focused": {"pane": pane, "error": error}}

    return step


async def capture(work: Path, only: str | None = None) -> tuple[list[dict], list[str]]:
    from scenarios_popup import SCENARIOS  # tools/ is on sys.path

    rec, clock = Recorder(), Clock()
    rec.install(clock)
    out, failures = [], []
    for scn, steps in SCENARIOS(sys.modules[__name__]):
        if only and not scn.name.startswith(only):
            continue
        try:
            doc = await run_scenario(scn, steps, work, rec, clock)
        except AssertionError as exc:
            failures.append(str(exc))
            print(f"FAILED {exc}", file=sys.stderr)
            continue
        out.append(doc)
    return out, failures


def write(path: Path, scenarios: list[dict]) -> None:
    """One step per line: a behaviour change shows as a small diff."""
    head = {"schema": SCHEMA, "source": "tools/capture_popup_transitions.py (tui.InboxApp under run_test; TZ=UTC)",
            "delays_ms": {"arm": 400, "row_arm": 1000, "grace": 500, "release": 500, "go": 500}}
    parts = []
    for scn in scenarios:
        steps = ",\n".join("      " + json.dumps(s, ensure_ascii=False, separators=(",", ":")) for s in scn["steps"])
        parts.append(f'    {{"name": {json.dumps(scn["name"])}, "doc": {json.dumps(scn["doc"], ensure_ascii=False)},'
                     f' "steps": [\n{steps}\n    ]}}')
    text = json.dumps(head, ensure_ascii=False)[:-1] + ', "scenarios": [\n' + ",\n".join(parts) + "\n]}\n"
    json.loads(text)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def main(argv: list[str] | None = None) -> int:
    from capture_goldens import isolate  # the goldens' isolated environment

    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--out", type=Path, default=GOLDEN)
    p.add_argument("--only", help="scenarios whose name starts with this (a partial golden, for debugging)")
    args = p.parse_args(argv)
    work = Path(tempfile.mkdtemp(prefix="inbox-popup-"))
    try:
        isolate(work)
        import os

        from tests.conftest import FakeHerdr

        fake = FakeHerdr(work)
        os.environ.update(HERDR_BIN_PATH=str(fake.bin), FAKE_HERDR_LOG=str(fake.log))
        from herdr_inbox import tabs

        tabs._config_dir.cache_clear()
        scenarios, failures = asyncio.run(capture(work, args.only))
        write(args.out, scenarios)
        print(f"{len(scenarios)} scenarios, {sum(len(s['steps']) for s in scenarios)} steps -> {args.out}")
        if failures:
            print(f"{len(failures)} scenario(s) failed: {failures}", file=sys.stderr)
            return 1
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
