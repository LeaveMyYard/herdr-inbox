"""Textual inbox, opened by herdr as a popup (``inbox tui``).

Modelled on Claude Code's AskUserQuestion prompt (strings, behaviour and colour roles
checked against the Claude Code 2.1.299 bundle). A *group* is one ``ask_operator``
call (1–4 questions):

- several open groups (or any ⚠ undelivered answer): a group list of multiline option
  rows, focused pane first; Enter opens, ``d`` dismisses a group (``y``), ``g`` goes to
  its agent, Esc closes;
- inside a group (or straight away when it is the only one): its questions as tabs and
  its own Submit tab ("Review your answers" / "You have not answered all questions" /
  "Ready to submit your answers?"); a lone question sends on Enter; Esc goes back;
- "Type something." is Claude Code's Other; "Chat about this" focuses the agent's pane;
  ``n`` adds notes; options may carry a ``preview``.

No background event changes what the next key does: list arrivals are appended, ended
rows and closed tabs become inert placeholders until the operator moves, cursors are
targets (never indexes), nothing joins an open group, and every view arms 0.4 s after
it appears.
"""

from __future__ import annotations

import concurrent.futures
import functools
import logging
import os
import sqlite3
import sys
import subprocess
import time
from datetime import datetime, timezone

from rich.markup import escape
from textual import events
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, HorizontalScroll, Vertical, VerticalScroll
from textual.message import Message
from textual.screen import ModalScreen, Screen
from textual.widgets import Input, Static

from . import deeplink, herdr, radar, remote, service, tabs, trace
from .store import AGENT_GONE, CLAIM_STALE, PENDING, Question, Store
from .store import now as store_now

log = logging.getLogger("herdr_inbox")

POLL_SECONDS = 2.0
# After a view change the operator did not cause, keys are dropped this long: whatever they
# were typing was meant for the view that just went away.
GRACE_SECONDS = 0.5
NAV_KEYS = ("tab", "shift+tab", "left", "right")
# A question view ignores Enter, digits and Space this long after it appears, however it was
# reached: a key already on its way can never answer a question the operator has not seen.
ARM_SECONDS = 0.4
# A list row that appeared while the popup was open ignores Enter and digits this long:
# a key aimed at the rows the operator saw must not land on one they have not read.
ROW_ARM_SECONDS = 1.0
OTHER = "Type something."
TICK = "\u2714"  # Claude Code's figures.tick on macOS/Linux
CHAT = "Chat about this"
BUSY = "The inbox database is busy; try again in a moment"


def guarded(fn):
    """UI actions never crash the popup on a busy database: they show a notice instead."""

    @functools.wraps(fn)
    def wrapper(self, *args, **kwargs):
        try:
            return fn(self, *args, **kwargs)
        except sqlite3.OperationalError as exc:
            if "locked" not in str(exc) and "busy" not in str(exc):
                raise
            self.app.notify(BUSY, severity="warning")

    return wrapper


# The asking tab's label is resolved live (the operator may have renamed it, and an unnamed
# tab's label is just its position), briefly cached so the popup stays fast.
RADAR_HINT = "herdr-radar is installed but ?N isn't in its sidebar — run the radar-sidebar action"
TAB_LABEL_TTL = 5.0
LIVE_TAB_LABELS = True  # the logic tests pin labels to what was stored at ask time
_tab_labels: dict[str, tuple[float, str | None]] = {}


def tab_name(q: Question | None, store: Store | None = None) -> str:
    """The question's tab label as herdr shows it now (our ❓ prefix stripped); the label
    stored at ask time if the tab is gone or herdr is unreachable."""
    if q is None:
        return "-"
    fallback = q.tab_label or q.pane_id or "no pane"
    if not q.tab_id or not LIVE_TAB_LABELS:
        return fallback
    cached = _tab_labels.get(q.tab_id)
    if cached and time.monotonic() - cached[0] < TAB_LABEL_TTL:
        label = cached[1]
    elif service._bg_queue is not None:
        # the popup: never a herdr call on the UI thread -- refresh in the background, show
        # what we have meanwhile (the next poll re-renders with the fresh label)
        if q.tab_id not in _refreshing:
            _refreshing.add(q.tab_id)
            service.later(functools.partial(_refresh_label, q.tab_id, store.path if store is not None else None))
        label = cached[1] if cached else None
    else:
        label = _read_label(q.tab_id, store)
        _tab_labels[q.tab_id] = (time.monotonic(), label)
    return label or fallback


_refreshing: set[str] = set()


def _read_label(tab_id: str, store: Store | None) -> str | None:
    label = herdr.tab_label(tab_id)
    if label is not None:
        marks = tabs.marks_for(store, tab_id) if store is not None else {tabs.DEFAULT_PREFIX}
        label = tabs.base_label(label, marks)
    return label


def _refresh_label(tab_id: str, path) -> None:
    try:
        if path is not None:
            with Store(path, timeout=2, maintenance=False) as own:
                label = _read_label(tab_id, own)
        else:
            label = _read_label(tab_id, None)
        _tab_labels[tab_id] = (time.monotonic(), label)
    finally:
        _refreshing.discard(tab_id)


def _gone(q: Question | None) -> bool:
    return bool(q and (q.delivery_error or "").startswith(AGENT_GONE))


def _pending(q: Question) -> bool:
    return bool(q.delivery_error and q.delivery_error.startswith(PENDING))


def _pending_reason(q: Question) -> str:
    return (q.delivery_error or "")[len(PENDING):]


def pending_text(reason: str, busy_for: float | None) -> str:
    """Why a pending answer waits, and when it goes: ``busy: …`` waits service.BUSY_GRACE
    then is typed anyway; ``blocked: …`` waits for the prompt/dialog to go."""
    if reason == service.BUSY_WAIT_IDLE:
        return "⚠ not delivered — agent busy; will retry when it is idle"
    if reason.startswith("requeued:"):
        return "⚠ not delivered earlier — will resend when the agent is idle"
    if reason == service.PANEL:
        return "waiting: the agent's task panel has focus (Esc there to close it)"
    if reason.startswith("busy:"):
        left = int(-(-(service.BUSY_GRACE - (busy_for or 0.0)) // 1))
        if left > 0:
            return f"agent busy — will send in {left} s"
        return "agent busy — sending at the next retry (every 10 s)"
    if reason.startswith("blocked:"):
        return "waiting: agent is at a prompt/dialog"
    if reason == "waiting: the operator is in that pane":
        return "waiting for you to leave that pane (it is focused)"
    return reason


def _pending_line(q: Question, store: Store | None) -> str:
    busy_for = service._busy_for(store, q.id) if store is not None else None
    return pending_text(_pending_reason(q), busy_for)


def _draft_line(q: Question) -> str:
    d = q.draft_json or {}
    return service.format_answer(d.get("choices") or [], d.get("notes"), d.get("text"))


def _typing(screen: Screen) -> bool:
    return isinstance(screen.focused, Input)


def _cursor(here: bool) -> str:
    # one fixed cell on every row, its own span (exported screenshots drop a span's leading spaces)
    return "[$inbox-suggestion]❯[/]" if here else "[$background] [/]"


def _close(app: App) -> None:
    """Esc: back to the group list, or close the popup there (drafts stay; never sends)."""
    app.call_next(app.action_back)


def _armed(widget) -> bool:
    """Views ignore Enter and digits for ARM_SECONDS after they appear (however reached),
    and while a deep-linked popup waits for the operator's first deliberate key."""
    try:
        held = getattr(widget.app, "held", False)
    except Exception:  # noqa: BLE001 - not mounted in an app
        held = False
    return not held and time.monotonic() >= getattr(widget, "armed_at", 0)


HELD_HINT = "Opened from another plugin — press Tab or → to start answering (Esc for the list)"
# A held deep link is released only by Tab/→ (as the empty popup's arrivals are), never by
# ↑/↓ (shell history under a popup that just appeared), and not within RELEASE_GRACE of
# the popup appearing; the releasing key does nothing else.
RELEASE_GRACE = 0.5


class QuestionView(Vertical, can_focus=True):
    """One question as Claude Code shows it: ❯ over numbered options, Type something., Chat about this."""

    class Answered(Message):
        def __init__(self, qid: int, detail: dict | None) -> None:
            super().__init__()
            self.qid, self.detail = qid, detail

    class Chat(Message):
        def __init__(self, q: Question) -> None:
            super().__init__()
            self.q = q

    BINDINGS = [
        Binding("up", "move(-1)", show=False),
        Binding("k", "move(-1)", show=False),
        Binding("down", "move(1)", show=False),
        Binding("j", "move(1)", show=False),
        Binding("enter", "select", show=False),
        Binding("space", "toggle", show=False),
        Binding("n", "note", show=False),
        Binding("x", "clear", show=False),
        Binding("escape", "back", show=False),
        *[Binding(str(n), f"jump({n})", show=False) for n in range(1, 10)],
    ]

    def __init__(self, q: Question, in_group: bool = False, last: bool = True):
        super().__init__()
        self.q = q
        self.in_group = in_group
        self.last = last
        self.options = q.options or []
        labels = [o["label"] for o in self.options]
        draft = q.draft_json or {}
        self.picked = {labels.index(c) for c in draft.get("choices") or [] if c in labels}
        self.notes = {labels.index(k): v for k, v in (draft.get("notes") or {}).items() if k in labels}
        self.text = draft.get("text")
        self.rows = [("opt", i) for i in range(len(self.options))] + [("other", None)]
        if q.multi_select:
            self.rows.append(("submit", None))
        else:
            self.rows.append(("chat", None))
        # ``rows`` is fixed for the life of this view (a question's options never change), so
        # ``cursor`` indexes an immutable list of row identities; nothing reloads it.
        self.cursor = self._initial_cursor()
        self.editing: tuple[str, int] | None = None  # ("note", option) or ("text", None)
        self.has_preview = any(o.get("preview") for o in self.options)

    def _initial_cursor(self) -> int:
        if self.text and not self.picked:
            return len(self.options)
        return min(self.picked) if self.picked else 0

    # -- layout --------------------------------------------------------------

    def compose(self) -> ComposeResult:
        q = self.q
        if q.header and not self.in_group:
            yield Static(f"[reverse b] {escape(q.header)} [/]", classes="chip")
        meta = f"{q.ref} · {tab_name(q, getattr(self.app, 'store', None))}" + (f" · {q.asked_by}" if q.asked_by else "")
        yield Static(escape(meta), classes="meta")
        yield Static(f"[b]{escape(q.question)}[/b]", classes="question")
        if q.context:
            yield Static(escape(q.context), classes="context")
        rows = [Static(id=f"row-{i}", classes="row") for i in range(len(self.rows))]
        if self.has_preview:
            with Horizontal(classes="split"):
                yield VerticalScroll(*rows, classes="choices")
                yield VerticalScroll(Static(id="preview", markup=False), classes="preview-box")
        else:
            yield VerticalScroll(*rows, classes="choices")
        yield Static(classes="hint")

    def on_mount(self) -> None:
        self.armed_at = time.monotonic() + ARM_SECONDS
        self.render_rows()

    def hint(self) -> str:
        if self.editing:
            return "Enter to save · Esc to cancel"
        nav = "Tab/Arrow keys to navigate" if self.in_group else "↑/↓ to navigate"
        pick = "Enter to toggle" if self.q.multi_select else "Enter to select"
        return f"{pick} · {nav} · press n to add notes · g go to agent · d dismiss · Esc to cancel"

    def set_mode(self, in_group: bool, last: bool) -> None:
        """Tabs came or went: update in place, never rebuild (typed text, picks, notes survive)."""
        if (in_group, last) != (self.in_group, self.last):
            self.in_group, self.last = in_group, last
            self.render_rows()

    def row_text(self, i: int) -> str:
        kind, opt_i = self.rows[i]
        here = i == self.cursor
        cur = _cursor(here)
        number = f"[$text-muted]{i + 1}.[/]"
        if kind == "submit":
            label = "Submit" if self.last else "Next"
            return f"{cur} [$text-muted]  [/] " + (f"[b $inbox-suggestion]{label}[/]" if here else f"[$foreground]{label}[/]")
        if kind == "chat":
            return f"{cur} {number} " + (f"[b $inbox-suggestion]{CHAT}[/]" if here else f"[$foreground]{CHAT}[/]")
        # Claude Code's roles: the focused row is "suggestion", a chosen answer "success" with
        # its tick (TICK); multi-select boxes are "[✔]" in success, "[ ]" otherwise
        box = ""
        if self.q.multi_select:
            on = (opt_i in self.picked) if kind == "opt" else bool(self.text)
            box = f"[$inbox-success]\\[{TICK}][/]\u00a0" if on else "\\[ ]\u00a0"
        if kind == "other":
            if self.text:
                label = f"[i]{escape(self.text)}[/i]"
            else:
                label = f"[dim]{OTHER}[/dim]"
            if here:
                label = f"[b $inbox-suggestion]{escape(self.text or OTHER)}[/]"
            tick = f"\u00a0[$inbox-success]{TICK}[/]" if self.text and not self.q.multi_select else ""
            return f"{cur} {number} {box}{label}{tick}"
        opt = self.options[opt_i]
        text = escape(opt["label"])
        tick = f"\u00a0[$inbox-success]{TICK}[/]" if opt_i in self.picked and not self.q.multi_select else ""
        lines = [f"{cur} {number} {box}" + (f"[b $inbox-suggestion]{text}[/]" if here else f"[$foreground]{text}[/]") + tick]
        if opt.get("description"):
            lines.append(f"      [dim]{escape(opt['description'])}[/]")
        if self.notes.get(opt_i):
            lines.append(f"      [$inbox-suggestion]Notes:[/] [i]{escape(self.notes[opt_i])}[/]")
        return "\n".join(lines)

    def render_rows(self) -> None:
        for i in range(len(self.rows)):
            self.query_one(f"#row-{i}", Static).update(self.row_text(i))
        self.query_one(".hint", Static).update(self.hint())
        # keep the highlighted row on screen (Enter sends it): a short popup scrolls to it
        self.query_one(f"#row-{self.cursor}").scroll_visible(animate=False)
        if self.has_preview:
            kind, opt_i = self.rows[self.cursor]
            preview = self.options[opt_i].get("preview") if kind == "opt" else None
            box = self.query_one("#preview", Static)
            box.update(preview or "No preview available")
            box.set_class(not preview, "empty")

    # -- actions -------------------------------------------------------------

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        # While a note or the text box is open, keys belong to it (Esc still cancels).
        if action != "back" and _typing(self.screen):
            return False
        if action in ("select", "jump", "toggle") and not _armed(self):
            return False  # not armed yet (see ARM_SECONDS), or a deep link still held
        return True

    def action_move(self, delta: int) -> None:
        self.cursor = (self.cursor + delta) % len(self.rows)
        self.render_rows()

    def action_jump(self, n: int) -> None:
        if 1 <= n <= len(self.rows) and self.rows[n - 1][0] != "submit":
            self.cursor = n - 1
            self.render_rows()

    def action_toggle(self) -> None:
        kind, opt_i = self.rows[self.cursor]
        if not self.q.multi_select:
            return
        if kind == "opt":
            self.picked ^= {opt_i}
        elif kind == "other":
            self._open_input("text")
            return
        self.render_rows()

    def action_select(self) -> None:
        kind, opt_i = self.rows[self.cursor]
        if kind == "chat":
            self.post_message(self.Chat(self.q))
        elif kind == "other":
            self._open_input("text")
        elif kind == "submit":
            if not self.picked and not self.text:
                self.notify("Select at least one option", severity="warning")
                return
            self._answer()
        elif self.q.multi_select:
            self.action_toggle()
        else:
            self.picked, self.text = {opt_i}, None
            self._answer()

    def action_note(self) -> None:
        kind, opt_i = self.rows[self.cursor]
        if kind == "opt":
            self._open_input("note", opt_i)

    def action_clear(self) -> None:
        self.picked, self.notes, self.text = set(), {}, None
        self.render_rows()
        if self.in_group:
            self.post_message(self.Answered(self.q.id, None))

    def action_back(self) -> None:
        if self.editing:
            self._close_input()
        else:
            _close(self.app)

    def _answer(self) -> None:
        labels = [self.options[i]["label"] for i in sorted(self.picked)]
        notes = {self.options[i]["label"]: n for i, n in self.notes.items()}
        self.render_rows()
        self.post_message(self.Answered(self.q.id, service.make_detail(self.text, labels, notes)))

    # -- inline inputs -------------------------------------------------------

    def _open_input(self, kind: str, opt_i: int | None = None) -> None:
        self._close_input()
        self.editing = (kind, opt_i)
        if kind == "note":
            value, placeholder = self.notes.get(opt_i, ""), "Notes"
            row = self.rows.index(("opt", opt_i))
        else:
            value, placeholder = self.text or "", OTHER
            row = self.rows.index(("other", None))
        box = Input(value=value, placeholder=placeholder, id="inline")
        self.query_one(".choices").mount(box, after=self.query_one(f"#row-{row}"))
        box.focus()
        self.render_rows()

    def _close_input(self) -> None:
        self.editing = None
        for box in self.query("#inline"):
            box.remove()
        self.focus()
        self.render_rows()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        event.stop()
        kind, opt_i = self.editing or ("", None)
        value = event.value.strip()
        self._close_input()
        if kind == "note":
            if value:
                self.notes[opt_i] = value
            else:
                self.notes.pop(opt_i, None)
            self.render_rows()
        elif kind == "text":
            if self.q.multi_select:  # Claude Code: the typed text is added to the picks
                self.text = value or None
                self.render_rows()
            elif value:  # single-select: the typed text is the answer
                self.picked, self.text = set(), value
                self._answer()


class ReviewView(Vertical, can_focus=True):
    """Review your answers: every question with its answer, then Submit answers / Cancel."""

    class Edit(Message):
        def __init__(self, qid: int) -> None:
            super().__init__()
            self.qid = qid

    class Submit(Message):
        def __init__(self, qids: list[int]) -> None:
            super().__init__()
            self.qids = qids

    BINDINGS = [
        Binding("up", "move(-1)", show=False),
        Binding("k", "move(-1)", show=False),
        Binding("down", "move(1)", show=False),
        Binding("j", "move(1)", show=False),
        Binding("enter", "select", show=False),
        Binding("1", "pick(0)", show=False),
        Binding("2", "pick(1)", show=False),
        Binding("x", "drop", show=False),
        Binding("escape", "back", show=False),
    ]

    def __init__(self, store: Store, qids: list[int]):
        super().__init__()
        self.store = store
        self.qids = qids
        self.items: list[Question] = []
        # The cursor is a target, never a row index: ("item", qid), "submit" or "cancel";
        # None is the inert position (nothing highlighted, Enter does nothing). Reloads
        # re-resolve it, so rows appearing or vanishing can never move it onto an action.
        self.cursor: tuple[str, int] | str | None = "submit"

    def compose(self) -> ComposeResult:
        yield VerticalScroll(Static(id="review-body"), classes="choices")
        yield Static(classes="hint")

    def on_mount(self) -> None:
        self.armed_at = time.monotonic() + ARM_SECONDS
        self.reload()

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        return action not in ("select", "pick") or _armed(self)

    @guarded
    def reload(self) -> None:
        self.items = [q for q in (self.store.get(i) for i in self.qids) if q and q.status == "open"]
        if self.cursor not in self.targets:  # its question went away: park, never on an action
            self.cursor = None
        self.render_rows()

    @property
    def targets(self) -> list:
        return [("item", q.id) for q in self.items] + ["submit", "cancel"]

    @property
    def drafted(self) -> list[Question]:
        return [q for q in self.items if q.draft_json]

    def render_rows(self) -> None:
        lines = ["[b]Review your answers[/b]", ""]
        for i, q in enumerate(self.items):
            mark = "[$inbox-suggestion]❯[/]" if self.cursor == ("item", q.id) else "[$foreground]●[/]"
            where = f"{q.ref} · {escape(tab_name(q, self.store))} · "
            lines.append(f"{mark} [$foreground]{where}{escape(service.first_line(q.question, 80))}[/]")
            if q.draft_json:
                lines.append(f"  [$inbox-success]→ {escape(_draft_line(q))}[/]")
            else:
                lines.append("  [dim]→ (not answered)[/dim]")
        if len(self.drafted) < len(self.items):
            lines += ["", "[$inbox-warning]⚠ You have not answered all questions[/]"]
        lines += ["", "[b]Ready to submit your answers?[/b]", ""]
        for k, (target, label) in enumerate((("submit", "Submit answers"), ("cancel", "Cancel"))):
            here = self.cursor == target
            text = f"[b $inbox-suggestion]{label}[/]" if here else f"[$foreground]{label}[/]"
            lines.append(f"{_cursor(here)} [$text-muted]{k + 1}.[/] {text}")
        self.query_one("#review-body", Static).update("\n".join(lines))
        self.query_one(".hint", Static).update(
            "Enter to select · ↑/↓ to navigate · Tab/Arrow keys to switch questions · Esc to cancel")

    def action_move(self, delta: int) -> None:
        targets = self.targets
        if self.cursor is None:  # from the inert position: the first row (or the last, going up)
            self.cursor = targets[0] if delta > 0 else targets[-1]
        else:
            self.cursor = targets[(targets.index(self.cursor) + delta) % len(targets)]
        self.render_rows()

    def action_pick(self, k: int) -> None:
        self.cursor = ("submit", "cancel")[k]
        self.action_select()

    def action_select(self) -> None:
        if self.cursor is None:
            return
        if isinstance(self.cursor, tuple):
            self.post_message(self.Edit(self.cursor[1]))
        elif self.cursor == "submit":
            if self.drafted:
                self.post_message(self.Submit([q.id for q in self.drafted]))
            else:
                self.notify("Nothing answered yet", severity="warning")
        else:
            _close(self.app)

    @guarded
    def action_drop(self) -> None:
        if isinstance(self.cursor, tuple):
            self.store.set_draft(self.cursor[1], None)
            self.reload()

    def action_back(self) -> None:
        _close(self.app)


VIEW_CSS = """
.chip { width: auto; margin-bottom: 1; }
.meta { color: $text-muted; }
.question { margin: 1 0; }
.context { color: $text-muted; margin-bottom: 1; }
.choices { height: 1fr; }
.split { height: 1fr; margin-bottom: 1; }
.split > .choices { width: 1fr; }
.preview-box { width: 1fr; border-left: solid $inbox-suggestion; padding: 0 1; }
#preview.empty { color: $text-muted; }
.row { height: auto; }
#inline { margin-left: 4; }
.hint { dock: bottom; color: $text-muted; height: 1; }
"""


class GoneView(Vertical, can_focus=True):
    """The current tab's question closed under the operator: an inert placeholder.

    Enter and number keys do nothing here; only an explicit tab move (or Esc) leaves, so
    keystrokes meant for this question can never land on another one.
    """

    BINDINGS = [Binding("escape", "back", show=False)]

    def __init__(self, ref: str, reason: str):
        super().__init__()
        self.ref, self.reason = ref, reason

    def compose(self) -> ComposeResult:
        yield Static(f"[b $inbox-warning]{self.ref} was closed[/] [dim]({escape(self.reason)})[/dim]", classes="question")
        yield Static("Nothing to answer here.", classes="meta")
        yield Static("Tab/Arrow keys to move to another question · Esc to cancel", classes="hint")

    def action_back(self) -> None:
        _close(self.app)


class FailedView(Vertical, can_focus=True):
    """An answer that was not delivered: retry it, or stop showing it here."""

    class Retry(Message):
        def __init__(self, qid: int) -> None:
            super().__init__()
            self.qid = qid

    class Hide(Message):
        def __init__(self, qid: int) -> None:
            super().__init__()
            self.qid = qid

    BINDINGS = [
        Binding("up", "move(-1)", show=False),
        Binding("down", "move(1)", show=False),
        Binding("enter", "select", show=False),
        Binding("1", "pick(0)", show=False),
        Binding("2", "pick(1)", show=False),
        Binding("escape", "back", show=False),
    ]

    def __init__(self, q: Question):
        super().__init__()
        self.q = q
        self.cursor = 0

    def compose(self) -> ComposeResult:
        q = self.q
        meta = f"{q.ref} · {tab_name(q, getattr(self.app, 'store', None))}" + (f" · {q.asked_by}" if q.asked_by else "")
        if _gone(q):
            state = (f"[$inbox-warning]⚠ {q.ref} agent closed — answer kept for check_answers[/]: "
                     f"[dim]{escape((q.delivery_error or '')[len(AGENT_GONE):])}[/dim]")
        elif _pending(q):
            state = (f"⏳ {q.ref} is already answered — waiting to be delivered[/]: "
                     f"[dim]{escape(_pending_line(q, getattr(self.app, 'store', None)))}[/dim]")
        else:
            state = f"[$inbox-warning]⚠ {q.ref} not delivered[/]: [dim]{escape(q.delivery_error or '')}[/dim]"
        yield Static(("[b]" + state) if _pending(q) else state,
                     classes="question")
        yield Static(escape(meta), classes="meta")
        yield Static(f"[b]{escape(q.question)}[/b]\n  [$inbox-success]→ {escape(q.answer or '')}[/]", classes="question")
        yield Static(id="failed-rows")
        yield Static("Enter to select · Tab/Arrow keys to navigate · g go to agent · Esc to cancel", classes="hint")

    def on_mount(self) -> None:
        self.armed_at = time.monotonic() + ARM_SECONDS
        self.render_rows()

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        return action not in ("select", "pick") or _armed(self)

    def render_rows(self) -> None:
        busy = _pending(self.q) and _pending_reason(self.q).startswith("busy:")
        labels = ("Send now (the agent is busy; the transcript check confirms it landed)" if busy else "Retry delivery",
                  "Stop showing it here (the agent can still fetch it with check_answers)")
        lines = []
        for k, label in enumerate(labels):
            here = k == self.cursor
            text = f"[b $inbox-suggestion]{label}[/]" if here else f"[$foreground]{label}[/]"
            lines.append(f"{_cursor(here)} [$text-muted]{k + 1}.[/] {text}")
        self.query_one("#failed-rows", Static).update("\n".join(lines))

    def action_move(self, delta: int) -> None:
        self.cursor = (self.cursor + delta) % 2
        self.render_rows()

    def action_pick(self, k: int) -> None:
        self.cursor = k
        self.action_select()

    def action_select(self) -> None:
        self.post_message((self.Retry if self.cursor == 0 else self.Hide)(self.q.id))

    def action_back(self) -> None:
        _close(self.app)


class ConfirmDismiss(ModalScreen[bool]):
    """One-key confirmation: only y dismisses; any other key keeps it."""

    CSS = """
    ConfirmDismiss { align: center middle; }
    #confirm { width: auto; max-width: 80%; padding: 1 2; border: round $inbox-warning; background: $panel; }
    """

    def __init__(self, what: str, detail: str):
        """``what``: e.g. "Q7" or "Q2–Q4 (3 questions)"; ``detail``: one dim line."""
        super().__init__()
        self.what, self.detail = what, detail

    def compose(self) -> ComposeResult:
        yield Static(f"Dismiss [b]{escape(self.what)}[/b] without answering?\n[dim]{escape(self.detail)}"
                     f"[/dim]\n\n[$inbox-warning]y[/] to dismiss · any other key to keep it", id="confirm")

    def on_key(self, event) -> None:
        event.stop()
        self.dismiss(event.key == "y")  # not d: a quick double d must not confirm


def _age(iso: str) -> str:
    from .cli import _age as age

    return age(iso)


Entry = tuple[str, int]  # ("group", group id) or ("failed", question id)

# Claude Code's own theme roles for its question prompt (read from the 2.1.299 bundle):
# the pointer and focused option use "suggestion", chosen answers (✓, the Review list's
# "→ answer") use "success", and "warning" is only for warnings ("You have not answered
# all questions"). Same roles, dark and light values as Claude Code's default themes.
CC_COLORS = {
    "dark": {"inbox-suggestion": "#b1b9f9", "inbox-success": "#4eba65", "inbox-warning": "#ffc107"},
    "light": {"inbox-suggestion": "#5769f7", "inbox-success": "#2c7a39", "inbox-warning": "#966c1e"},
}
SUBMIT = ("submit", 0)


def entry_order(questions: list[Question], failed: list[Question], focus_pane: str | None) -> list[Entry]:
    """Groups from the focused pane first, then the rest oldest first; ⚠ entries after them."""
    groups: dict[int, list[Question]] = {}
    for q in sorted(questions, key=lambda q: q.id):
        groups.setdefault(q.group, []).append(q)
    ordered = sorted(groups.values(), key=lambda g: (g[0].pane_id != focus_pane, g[0].id))
    return [("group", g[0].group) for g in ordered] + [("failed", q.id) for q in failed]


def tab_order(questions: list[Question], focus_pane: str | None) -> list[int]:
    """Question ids in entry order (kept for callers that order single questions)."""
    groups: dict[int, list[Question]] = {}
    for q in sorted(questions, key=lambda q: q.id):
        groups.setdefault(q.group, []).append(q)
    ordered = sorted(groups.values(), key=lambda g: (g[0].pane_id != focus_pane, g[0].id))
    return [q.id for g in ordered for q in g]


class GroupListView(Vertical, can_focus=True):
    """Every open group (one ask_operator call) and every undelivered answer, as Claude Code
    option rows. The cursor is a target (an entry), never a row index; a row whose group
    closed stays in place as an inert placeholder until the operator moves the cursor."""

    class Open(Message):
        def __init__(self, entry: Entry) -> None:
            super().__init__()
            self.entry = entry

    BINDINGS = [
        Binding("up", "move(-1)", show=False),
        Binding("k", "move(-1)", show=False),
        Binding("down", "move(1)", show=False),
        Binding("j", "move(1)", show=False),
        Binding("enter", "select", show=False),
        Binding("escape", "back", show=False),
        *[Binding(str(n), f"jump({n})", show=False) for n in range(1, 10)],
    ]

    def __init__(self, inbox: "InboxApp", cursor: Entry | None):
        super().__init__()
        self.inbox = inbox
        self.cursor = cursor

    def compose(self) -> ComposeResult:
        yield Static(id="list-title", classes="question")
        # one widget per row, so the cursor's row can be scrolled into view
        yield VerticalScroll(Static("No open questions", id="list-empty"), classes="choices", id="list-rows")
        yield Static("Enter to open · ↑/↓ to navigate · g go to agent · d dismiss · Esc to cancel", classes="hint")

    def on_mount(self) -> None:
        self.armed_at = time.monotonic() + ARM_SECONDS
        if self.cursor not in self.inbox.entries:  # its row is gone: the inert rest position
            self.cursor = None
        self.render_rows()

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        return action not in ("select", "jump") or _armed(self)

    def row_text(self, n: int, entry: Entry) -> str:
        here = entry == self.cursor
        cur, number = _cursor(here), f"[$text-muted]{n}.[/]"
        closed = self.inbox.closed.get(entry)
        store = self.inbox.store
        if entry[0] == "failed":
            q = store.get(entry[1])
            where = escape(tab_name(q, store))
            if closed:
                return f"{cur} {number} [$inbox-warning]✕ Q{entry[1]} · {where} · {escape(closed)}[/]"
            text = _pending_line(q, store) if _pending(q) else ""
            if _gone(q):  # nothing to do here: the answer waits for check_answers (d hides it)
                head = f"⚠ Q{q.id} agent closed — answer kept for check_answers · {where}"
                line1 = f"[b $inbox-warning]{head}[/]" if here else f"[$inbox-warning]{head}[/]"
            elif _pending(q) and text.startswith("⚠ "):  # answered; it already failed once
                head = f"⚠ Q{q.id} answered · {escape(text[2:])} · {where}"
                line1 = f"[b $inbox-warning]{head}[/]" if here else f"[$inbox-warning]{head}[/]"
            elif _pending(q):  # answered, waiting for the agent: information, not a warning
                head = f"⏳ Q{q.id} answered · waiting to deliver: {escape(text)} · {where}"
                line1 = f"[b $inbox-suggestion]{head}[/]" if here else f"[$foreground]{head}[/]"
            else:
                head = f"⚠ Q{q.id} not delivered · {where} · {escape(q.delivery_error or 'no attempt recorded')}"
                line1 = f"[b $inbox-warning]{head}[/]" if here else f"[$inbox-warning]{head}[/]"
            return f"{cur} {number} {line1}\n      [dim]{escape(service.first_line(q.question, 60))} → " \
                f"{escape(service.first_line(q.answer or '', 40))}[/dim]"
        if entry[0] == "remote":
            return self._remote_row(cur, number, here, entry, closed)
        if entry[0] == "routbox":
            return self._outbox_row(cur, number, here, entry, closed)
        if entry[0] == "rproblem":
            return self._problem_row(cur, number, here, entry, closed)
        qs = store.group(entry[1], include_closed=True)
        first = qs[0] if qs else None
        where = escape(tab_name(first, store))
        live_qs = [q for q in qs if q.status == "open"]
        name = escape(service.group_name(live_qs or qs)) if qs else where
        if closed or not first:
            return f"{cur} {number} [$inbox-warning]✕ {name} · {where} · {escape(closed or 'closed')}[/]"
        live = [q for q in qs if q.status == "open"]
        drafted = sum(1 for q in live if q.draft_json)
        # line 1: the call's name (its title, see service.group_name) and its state
        head = f"{name} · {len(live)} question{'s' if len(live) != 1 else ''}"
        line1 = f"{cur} {number} " + (f"[b $inbox-suggestion]{head}[/]" if here else f"[$foreground]{head}[/]")
        if drafted:  # the trailing space stays inside the coloured run (SVG export drops leading ones)
            line1 += f" [$inbox-success]· {drafted} drafted [/]"
        else:
            line1 += " "
        line1 += f"[dim]· {_age(first.created_at)}[/dim]"
        # line 2: who asked, and the questions' header chips
        chips = " · ".join(escape(q.header or service.first_line(q.question, 30)) for q in live)
        if len(live) == 1 and chips == name:  # a lone untitled question: its name already says it
            chips = ""
        detail = " · ".join([where] + ([escape(first.asked_by)] if first.asked_by else []) + ([chips] if chips else []))
        return f"{line1}\n      [dim]{detail}[/dim]"

    def _remote_row(self, cur: str, number: str, here: bool, entry: Entry, closed: str | None) -> str:
        found = remote.group_by_key(self.inbox.store, entry[1])
        if closed or not found:
            mid = str(entry[1]).split("|")[0]
            row = self.inbox.store.db.execute("SELECT label FROM remote_machines WHERE machine_id = ?", (mid,)).fetchone()
            return f"{cur} {number} [dim]✓ {escape(row[0] if row and row[0] else mid)} · {escape(closed or 'closed')}[/dim]"
        m, qs = found
        first, label = qs[0], escape(m.get("label") or m["machine_id"])
        name = escape(first.get("title") or first.get("header") or service.first_line(first["question"], 80))
        head = f"{label}:Q{first['id']} {name} · {len(qs)} question{'s' if len(qs) != 1 else ''}"
        line1 = f"{cur} {number} " + (f"[b $inbox-suggestion]{head}[/]" if here else f"[$foreground]{head}[/]")
        line1 += f" [dim]· {_age(first.get('created_at') or '')}[/dim]" if first.get("created_at") else ""
        detail = [label] + [escape(x) for x in (first.get("tab_label"), first.get("asked_by")) if x]
        note = remote.offline_note(m)
        if note:
            detail.append(f"[$inbox-warning]{escape(note)}[/]")
        return f"{line1}\n      [dim]{' · '.join(detail)}[/dim]"

    def _outbox_row(self, cur: str, number: str, here: bool, entry: Entry, closed: str | None) -> str:
        row = next((r for r in remote.outbox_rows(self.inbox.store) if r["request_id"] == entry[1]), None)
        if closed or row is None:
            return f"{cur} {number} [dim]✓ {escape(closed or 'sent')}[/dim]"
        label = escape(row.get("label") or row["machine_id"])
        ref = f"{label}:Q{row['remote_id']}"
        if row["state"] == "queued" and row.get("sending_at"):
            head = f"⏳ {ref} {row['action']} · sending to {label} now"
            return f"{cur} {number} " + (f"[b $inbox-suggestion]{escape(head)}[/]" if here else f"[$foreground]{escape(head)}[/]")
        if row["state"] == "queued" and row["expires_at"] <= datetime.now(timezone.utc).isoformat(timespec="seconds"):
            head = f"⚠ {ref} {row['action']} expired · not sent to {label} · d clears"  # until the sweep settles it
            return f"{cur} {number} " + (f"[b $inbox-warning]{escape(head)}[/]" if here else f"[$inbox-warning]{escape(head)}[/]")
        if row["state"] == "queued":
            until = datetime.fromisoformat(row["expires_at"]).astimezone().strftime("%H:%M")
            head = f"⏳ {ref} {row['action']} queued · sending to {label} · expires {until}"
            return f"{cur} {number} " + (f"[b $inbox-suggestion]{escape(head)}[/]" if here else f"[$foreground]{escape(head)}[/]")
        head = f"⚠ {ref} {row['action']} {row['state']}: {row.get('result') or ''} · d clears"
        return f"{cur} {number} " + (f"[b $inbox-warning]{escape(head)}[/]" if here else f"[$inbox-warning]{escape(head)}[/]")

    def _problem_row(self, cur: str, number: str, here: bool, entry: Entry, closed: str | None) -> str:
        mid, _, rid = str(entry[1]).partition("|")
        p = next((p for p in remote.problem_rows(self.inbox.store)
                  if p["machine_id"] == mid and str(p["remote_id"]) == rid), None)
        if closed or p is None:
            return f"{cur} {number} [dim]✓ {escape(closed or 'cleared')}[/dim]"
        head = f"⚠ {p['label']}:Q{p['remote_id']} answered · {p['why']}"
        line1 = f"{cur} {number} " + (f"[b $inbox-warning]{escape(head)}[/]" if here else f"[$inbox-warning]{escape(head)}[/]")
        q = p["data"]
        return (f"{line1}\n      [dim]{escape(service.first_line(q.get('question') or '', 60))} → "
                f"{escape(service.first_line(q.get('answer') or '', 40))} · d clears[/dim]")

    def render_rows(self) -> None:
        if not self.query("#list-title"):
            return  # a poll can land while this view is still mounting or already leaving
        live = self.inbox.live_entries
        groups = sum(1 for e in live if e[0] == "group")
        others = [self.inbox.store.get(e[1]) for e in live if e[0] == "failed"]
        pending = sum(1 for q in others if q and _pending(q))
        failed = len(others) - pending
        parts = [f"{groups} group{'s' if groups != 1 else ''}"]
        parts += [f"{pending} pending delivery"] if pending else []
        parts += [f"{failed} not delivered"] if failed else []
        self.query_one("#list-title", Static).update(f"[b]Open questions[/b] [dim]· {' · '.join(parts)}[/dim]")
        entries = self.inbox.entries
        box = self.query_one("#list-rows", VerticalScroll)
        rows = list(box.query(".entry"))
        if len(rows) < len(entries):
            new = [Static(classes="entry") for _ in range(len(entries) - len(rows))]
            box.mount_all(new)
            rows += new
        for extra in rows[len(entries):]:
            extra.remove()
        for n, (widget, entry) in enumerate(zip(rows, entries), 1):
            widget.update(self.row_text(n, entry))
            if entry == self.cursor:
                self.call_after_refresh(widget.scroll_visible, animate=False)
        self.query_one("#list-empty", Static).display = not entries

    def body_text(self) -> str:
        """All rows as rendered plain text (tests and debugging)."""
        return "\n".join(str(w.render()) for w in self.query(".entry"))

    def fresh(self, entry: Entry | None) -> bool:
        """The row arrived in the background less than ROW_ARM_SECONDS ago."""
        arrived = self.inbox.arrived_at.get(entry)
        return arrived is not None and time.monotonic() - arrived < ROW_ARM_SECONDS

    def _moved(self) -> None:
        """A deliberate cursor move: closed placeholders drop out now, the target stays."""
        self.inbox.purge_closed(keep=self.cursor)
        self.render_rows()

    def action_move(self, delta: int) -> None:
        entries = self.inbox.entries
        if not entries:
            return
        if self.cursor not in entries:  # the inert rest position: resume from the top/bottom
            self.cursor = entries[0] if delta > 0 else entries[-1]
        else:  # no wrap-around: ↓ on the last row stays, so an arrival is never where ↓ wraps to
            i = entries.index(self.cursor) + delta
            self.cursor = entries[max(0, min(i, len(entries) - 1))]
        self._moved()

    def action_jump(self, n: int) -> None:
        if 1 <= n <= len(self.inbox.entries) and not self.fresh(self.inbox.entries[n - 1]):
            self.cursor = self.inbox.entries[n - 1]
            self._moved()

    def action_select(self) -> None:
        if self.cursor in self.inbox.entries and self.cursor not in self.inbox.closed and not self.fresh(self.cursor):
            self.post_message(self.Open(self.cursor))

    def action_back(self) -> None:
        self.app.call_next(self.inbox.action_quit)


# The popup records answers and hands their delivery to a detached `inbox deliver` process,
# so it can close at once (a delivery can take seconds: herdr typing, the 15 s transcript
# check, tab labels). The test suite turns this off and delivers in a thread instead.
DETACHED_SEND = True
ENABLE_BACKGROUND = True  # side effects (tab labels, markers) off the UI thread; tests: inline


def spawn_delivery(qids: list[int], force: bool = False) -> None:
    """Start ``inbox deliver <ids>`` detached (it outlives the popup; claims serialize it)."""
    from . import worker

    args = [sys.executable, "-m", "herdr_inbox.cli", "deliver", *map(str, qids)] + (["--force"] if force else [])
    log_path = worker.db_path().parent / "worker.log"
    with open(log_path, "a") as err:
        subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=err,
                         start_new_session=True, close_fds=True, env=worker._spawn_env())


def spawn_verify(job: dict) -> None:
    """Start ``inbox verify <job>`` detached: the transcript check for answers this popup typed.
    If it cannot start, the claims go stale (60 s) and the worker takes the rows over, looking
    at the transcript before typing anything again."""
    import json as _json

    from . import worker

    args = [sys.executable, "-m", "herdr_inbox.cli", "verify", _json.dumps(job)]
    log_path = worker.db_path().parent / "worker.log"
    try:
        with open(log_path, "a") as err:
            subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=err,
                             start_new_session=True, close_fds=True, env=worker._spawn_env())
    except OSError:
        log.warning("could not start the transcript check; the worker will take it over", exc_info=True)


class AlreadySending(Exception):
    """Send now on an answer another process is delivering right now."""


@trace.traced("popup.after_submit")
def _after_submit(app: "InboxApp", done: list[Question], wanted: int, detached: bool = False) -> None:
    failed = [] if detached else [q for q in done if not q.delivered_at]
    if not done:
        app.notify("Nothing sent: those questions are no longer open", severity="warning")
    elif detached:
        app.notify(f"Sending {', '.join(q.ref for q in done)} (in the background; see ⏳/⚠ here if it waits)")
    elif failed:
        app.notify(f"Sent {', '.join(q.ref for q in done)}; not delivered: "
                   f"{', '.join(f'{q.ref} ({q.delivery_error})' for q in failed)}", severity="warning")
    else:
        app.notify(f"Sent {', '.join(q.ref for q in done)}")
    if done and len(done) < wanted:
        app.notify(f"{wanted - len(done)} answered question(s) had closed meanwhile", severity="warning")
    app.poll()
    if app.mode in ("list", "empty") and not app.live_entries and done and not failed:
        app.exit()  # like Claude Code's prompt: gone once everything is answered


def _send(app: "InboxApp", store: Store, qids: list[int], work, force: bool = False) -> None:
    """Record in a background thread, then deliver detached (DETACHED_SEND) or in that
    thread: the popup never waits for herdr, the tab-label lock or the transcript check.
    ``work(store, deliver)`` records the answers (and delivers them if ``deliver``)."""
    app.notify(f"Sending {', '.join(f'Q{q}' for q in qids)}…")

    def report(fn, *args, **kwargs) -> None:
        # The popup may be gone by now ("go to agent", Esc): the send itself is done and
        # recorded. Only "the app has stopped" is swallowed -- any other error, or one while
        # the app still runs, surfaces.
        if not app.is_running:
            return
        try:
            app.call_from_thread(fn, *args, **kwargs)
        except (RuntimeError, concurrent.futures.CancelledError):
            if app.is_running:
                raise

    def run() -> None:
        try:
            with trace.step("popup.send_thread"), Store(store.path) as own:  # sqlite connections stay in the thread that made them
                done = work(own, not detached)
                if detached and done:
                    service._ensure_worker()  # first: if this process dies before the spawn, the worker adopts it
                    # the pane the popup returns to (the operator's): type NOW, while the popup
                    # still holds the keyboard -- after it closes the operator may be typing
                    # there. Only the transcript check is handed off (inbox verify).
                    here = [q for q in done if q.pane_id and q.pane_id == app.focus_pane]
                    if here:
                        service.deliver_batch(own, here, force=force, verify_later=spawn_verify)
                    elsewhere = [q.id for q in done if q not in here]
                    if elsewhere:
                        try:
                            spawn_delivery(elsewhere, force=force)
                        except OSError:
                            log.warning("could not start the delivery; the worker will retry", exc_info=True)
        except AlreadySending as busy:
            report(app.notify, f"Q{busy.args[0]} is already being delivered", severity="warning")
            return
        except sqlite3.OperationalError:
            report(app.notify, BUSY, severity="warning")
            return
        report(_after_submit, app, done, len(qids), detached)

    detached = DETACHED_SEND

    app.run_worker(run, thread=True, group="send")


class InboxApp(App):
    """A group list (one row per ask_operator call, plus ⚠ undelivered answers), and inside a
    group its questions as tabs with its own Submit tab.

    The popup opens straight into the group when there is exactly one open group (and no ⚠
    entry); otherwise on the list. Background polls never change what the next key does:
    new list rows are appended without moving the cursor's target, closed rows and closed
    question tabs turn into inert placeholders until the operator moves, nothing is ever
    added to an open group, and every view ignores Enter/digits for ARM_SECONDS after it
    appears. Whether Enter sends at once (a lone question) is decided when the operator
    opens the group (or when the popup opens straight into it).
    """

    TITLE = "herdr inbox"
    CSS = """
    Screen { padding: 1 2; }
    #group-title { height: auto; margin-bottom: 1; text-style: bold; }
    #chips { height: 1; margin-bottom: 1; scrollbar-size-horizontal: 0; }
    #chips > Static { width: auto; margin-right: 2; }
    #chips > .sep { color: $text-muted; }
    #chips > .warn { color: $inbox-warning; }
    #empty { display: none; }
    InboxApp.empty #empty { display: block; }
    InboxApp.empty #chips, InboxApp.empty #body { display: none; }
    #radar-hint { display: none; height: auto; color: $text-muted; }
    #held-hint { display: none; height: auto; color: $inbox-suggestion; text-style: bold; }
    InboxApp.held #held-hint { display: block; }
    InboxApp.radar-missing #radar-hint { display: block; }
    """ + VIEW_CSS
    BINDINGS = [
        Binding("left", "switch(-1)", show=False),
        Binding("shift+tab", "switch(-1)", show=False, priority=True),
        Binding("right", "switch(1)", show=False),
        Binding("tab", "switch(1)", show=False, priority=True),
        Binding("d", "dismiss", show=False),
        Binding("g", "goto", show=False),
        Binding("escape", "back", show=False),
    ]

    def get_css_variables(self) -> dict[str, str]:
        variables = super().get_css_variables()
        variables.update(CC_COLORS["dark" if self.current_theme.dark else "light"])
        return variables

    def __init__(self, store: Store | None = None, focus_pane: str | None = None):
        super().__init__()
        self.store = store or Store()
        self.focus_pane = focus_pane
        self.mode = "empty"  # "empty" | "list" | "group" | "failed"
        self.entries: list[Entry] = []
        self.closed: dict[Entry, str] = {}  # list rows whose group/answer ended: inert placeholders
        self.arrived_at: dict[Entry, float] = {}  # rows added by a poll (see ROW_ARM_SECONDS)
        self.list_cursor: Entry | None = None
        self.direct = False  # opened straight into the only group: Esc closes the popup
        self.held = False  # deep-linked: Enter/digits wait for a navigation key (deeplink.py)
        self._unmounted = False  # set by on_unmount: a late poll tick does nothing
        # the open group
        self.entry: Entry | None = None
        self.tabs: list[tuple[str, int]] = []
        self.current: tuple[str, int] | None = None
        self.gone: dict[tuple[str, int], str] = {}  # tabs whose question closed while shown
        self.single = False  # Enter sends at once; decided when the group is opened
        self.grace_until = 0.0  # keys before this are dropped (see GRACE_SECONDS)

    @property
    def base(self) -> Screen:
        return self.screen_stack[0]

    # -- state -------------------------------------------------------------------

    def _live(self) -> tuple[list[Question], list[Question]]:
        return self.store.list(), self.store.failed_deliveries()

    def _remote_entries(self) -> list[Entry]:
        """Remote machines' open groups and queued/failed actions (the local mirror only)."""
        try:
            return ([("remote", key) for key, _, _ in remote.open_groups(self.store)]
                    + [("routbox", r["request_id"]) for r in remote.outbox_rows(self.store)]
                    + [("rproblem", f"{p['machine_id']}|{p['remote_id']}") for p in remote.problem_rows(self.store)])
        except sqlite3.Error:
            return []

    @property
    def live_entries(self) -> list[Entry]:
        return [e for e in self.entries if e not in self.closed]

    @property
    def open_qids(self) -> list[int]:
        return [i for k, i in self.tabs if k == "q" and (k, i) not in self.gone]

    @property
    def all_tabs(self) -> list[tuple[str, int]]:
        return self.tabs + ([] if self.single or not self.open_qids else [SUBMIT])

    def purge_closed(self, keep: Entry | None = None) -> None:
        """Drop closed placeholders from the list (on the operator's own move)."""
        self.entries = [e for e in self.entries if e not in self.closed or e == keep]
        self.closed = {e: r for e, r in self.closed.items() if e == keep}

    def compose(self) -> ComposeResult:
        yield Static(id="group-title")
        yield HorizontalScroll(id="chips")
        yield Vertical(id="body")
        yield Static("No open questions", id="empty")
        yield Static(RADAR_HINT, id="radar-hint")
        yield Static(HELD_HINT, id="held-hint")

    def check_radar(self) -> None:
        """Read-only: radar's sidebar lacks $inbox_q → one hint line. Never edits anything."""
        def run() -> None:
            try:
                if radar.missing():
                    self.call_from_thread(self.add_class, "radar-missing")
            except Exception:  # noqa: BLE001 - a hint is never worth an error
                pass
            try:
                from . import worker

                worker.ensure()  # the delivery worker retries what this popup answers
            except Exception:  # noqa: BLE001
                pass

        self.run_worker(run, thread=True, group="radar")

    @trace.traced("popup.on_mount")
    def on_mount(self) -> None:
        if ENABLE_BACKGROUND:
            service.enable_background()  # tab labels, markers: never on the UI thread
        self._popup_lock = deeplink.hold_popup_lock()  # `inbox open` reports an open popup
        self.check_radar()
        if self.focus_pane is None:
            self.focus_pane = (herdr.invocation_pane() or os.environ.get("INBOX_FOCUS_PANE")
                               or herdr.focused_pane())
        try:
            open_qs, failed = self._live()
        except sqlite3.OperationalError:
            open_qs, failed = [], []
            self.notify(BUSY, severity="warning")
        self.entries = entry_order(open_qs, failed, self.focus_pane) + self._remote_entries()
        target = None
        try:
            request = deeplink.consume_request()  # `inbox open --question …` (deeplink.py)
        except Exception:  # noqa: BLE001 - a deep link is never worth a broken popup
            request = None
        if request:
            target = deeplink.resolve(self.store, request)
            if target is None:
                self.notify(f"{deeplink.describe(request)} is no longer open")
        if target and target[0] in self.entries:
            # Another plugin opened us, maybe under the operator's typing: nothing sends
            # until a deliberate Tab/→ (see RELEASE_GRACE). Esc goes to the list.
            self.held, self.held_at = request.get("hold") is not False, time.monotonic()
            if self.held:  # `inbox open --no-hold`: the caller says an operator key opened it
                self.add_class("held")
            self.open_group(target[0], start_qid=target[1])
        elif not self.entries:
            self.show_empty()
        elif len(self.entries) == 1 and self.entries[0][0] == "group":
            self.direct = True
            self.open_group(self.entries[0])
        else:
            self.show_list(self.entries[0])
        self._poll_timer = self.set_interval(POLL_SECONDS, self.poll)

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        if isinstance(self.focused, Input):
            return action not in ("switch", "dismiss", "goto")
        if action == "switch":
            return self.mode == "group" or (self.mode == "empty" and bool(self.entries))
        if action in ("dismiss", "goto"):
            if self.mode == "list":
                cur = self._list_view().cursor if self._list_view() else None
                return cur is not None and cur in self.entries and cur not in self.closed
            if self.mode == "group":  # only on a live question tab (never on a placeholder)
                return self.current is not None and self.current != SUBMIT and self.current not in self.gone
            return action == "goto" and self.mode == "failed"
        return True

    async def on_event(self, event: events.Event) -> None:
        if isinstance(event, events.Key) and trace.enabled():
            trace.record("popup.key", 0, key=event.key)
        # The general rule: a background event never changes what the next keypress does.
        # Right after a view change the operator did not cause, keys are dropped.
        if isinstance(event, events.Key) and time.monotonic() < self.grace_until:
            event.stop()
            event.prevent_default()
            return
        await super().on_event(event)

    # -- views ---------------------------------------------------------------------

    def _mount(self, view) -> None:
        self.set_class(self.mode == "empty", "empty")
        body = self.base.query_one("#body")
        body.remove_children()
        if view is not None:
            body.mount(view)
            view.focus()

    def _list_view(self) -> GroupListView | None:
        found = self.base.query(GroupListView)
        return found.first() if found else None

    def show_empty(self) -> None:
        self.mode = "empty"
        self.render_chips()
        self._mount(None)
        self.render_empty()

    def render_empty(self) -> None:
        n = len(self.live_entries)
        text = "No open questions" if not n else f"{n} new question group{'s' if n > 1 else ''} — press Tab/→ to view"
        self.base.query_one("#empty", Static).update(text)

    def show_list(self, cursor: Entry | None = None) -> None:
        """The group list (an operator action, or the popup opening)."""
        self.mode, self.entry, self.direct = "list", None, False
        self.tabs, self.current, self.gone = [], None, {}
        if not self.entries:
            self.show_empty()
            return
        self.render_chips()
        self._mount(GroupListView(self, cursor if cursor is not None else self.list_cursor))

    def open_group(self, entry: Entry, start_qid: int | None = None) -> None:
        """Enter a group (the operator chose it, or it is the only one): its questions as tabs."""
        self.list_cursor = entry
        if entry[0] in ("routbox", "rproblem"):
            self.notify("This is on a remote machine; d clears it here")
            return
        if entry[0] == "remote":
            found = remote.group_by_key(self.store, entry[1])
            if not found:
                self.show_list(entry)
                return
            m, qs = found
            self.mode, self.entry, self.remote_open = "remote", entry, (m, qs[0])
            self.tabs, self.current = [], None
            self.render_chips()
            box = self.base.query_one("#group-title", Static)
            box.display = True
            box.update(escape(f"{m.get('label')}:Q{qs[0]['id']} · "
                              f"{qs[0].get('title') or service.first_line(qs[0]['question'], 80)}"))
            self._mount(QuestionView(remote.as_question(qs[0])))
            return
        if entry[0] == "failed":
            self.mode, self.entry = "failed", entry
            self.render_chips()
            self._mount(FailedView(self.store.get(entry[1])))
            return
        qs = [q for q in self.store.group(entry[1]) if q.status == "open"]
        if not qs:
            self.show_list(entry)
            return
        self.mode, self.entry = "group", entry
        self.tabs = [("q", q.id) for q in qs]
        self.gone = {}
        self.single = len(qs) == 1  # decided here, on the operator's move
        start = next((t for t in self.tabs if not self.store.get(t[1]).draft_json), self.tabs[0])
        if start_qid is not None and ("q", start_qid) in self.tabs:
            start = ("q", start_qid)
        self.show(start)

    def show(self, tab: tuple[str, int] | None) -> None:
        """Mount the view for a tab of the open group (operator actions only)."""
        self.current = tab
        if tab is None:
            self.back_to_list()
            return
        kind, qid = tab
        if tab == SUBMIT:
            view = ReviewView(self.store, self.open_qids)
        elif tab in self.gone:
            view = GoneView(f"Q{qid}", self.gone[tab])
        else:
            view = QuestionView(self.store.get(qid), in_group=not self.single, last=self._last_open(qid))
        self._mount(view)
        self.render_chips()

    def _last_open(self, qid: int) -> bool:
        return self.single or (self.open_qids[-1:] == [qid])

    def render_chips(self) -> None:
        self.render_group_title()
        row = self.base.query_one("#chips", HorizontalScroll)
        row.remove_children()
        tabs = self.all_tabs if self.mode == "group" else []
        if len(tabs) <= 1:
            row.display = False  # no empty band above the list or a lone question
            return
        first = self.store.get(self.tabs[0][1])
        chips = [Static(f"│ {escape(tab_name(first, self.store))}",
                        classes="sep")]
        current = None
        for tab in tabs:
            here = tab == self.current
            # The mark sits outside the highlighted label: a coloured mark inside a reversed
            # span would paint a coloured block.
            if tab == SUBMIT:
                mark, label, classes = "", "Submit", ""
            elif tab in self.gone:
                mark, label, classes = "✕\u00a0", f"Q{tab[1]}", "warn"
            else:
                q = self.store.get(tab[1])
                mark = f"[$inbox-success]{TICK}\u00a0[/]" if q and q.draft_json else "●\u00a0"
                label, classes = escape(q.header or q.ref), ""
            chip = Static(f"{mark}[reverse b] {label} [/]" if here else f"{mark}{label}",
                          classes=" ".join(c for c in (classes, "current" if here else "") if c))
            chips.append(chip)
            if here:
                current = chip
        row.display = True
        row.mount_all(chips)
        if current is not None:
            self.call_after_refresh(current.scroll_visible, animate=False)

    def render_group_title(self) -> None:
        """The open group's name above its tabs: its title, or for several untitled questions
        the fallback name (a lone untitled question already shows its header chip)."""
        box = self.base.query_one("#group-title", Static)
        qs = self.store.group(self.entry[1], include_closed=True) if self.mode == "group" and self.entry else []
        show = bool(qs) and (bool(qs[0].group_title) or len(qs) > 1)
        box.display = show
        live = [q for q in qs if q.status == "open"]
        box.update(escape(service.group_name(live or qs)) if show else "")

    def _gone_reason(self, kind: str, qid: int) -> str:
        q = self.store.get(qid)
        if kind == "failed":
            return "delivered" if q and q.delivered_at else "no longer pending"
        if q is None:
            return "removed"
        return {"withdrawn": "withdrawn by the agent", "answered": "answered",
                "dismissed": "dismissed"}.get(q.status, q.status)

    @guarded
    @trace.traced("popup.poll")
    def poll(self) -> None:
        """Arrivals become new list rows at the end (the cursor's target does not move);
        ended rows and ended question tabs become inert placeholders; nothing is added to an
        open group; no view is rebuilt unless the one on screen itself ended."""
        if len(self.screen_stack) > 1:  # a confirmation is open
            return
        if self._unmounted or not self.screen_stack:
            return  # tearing down (a late timer tick): nothing to update
        open_qs, failed = self._live()
        current = entry_order(open_qs, failed, None) + self._remote_entries()
        live = set(current)
        for e in self.entries:
            if e not in live and e not in self.closed:
                self.closed[e] = self._entry_reason(e)
        known = set(self.entries)
        new = [e for e in current if e not in known]
        now = time.monotonic()
        self.arrived_at.update({e: now for e in new})
        self.entries += new
        if self.mode == "empty":
            self.render_empty()
        elif self.mode == "list":
            view = self._list_view()
            if view:
                view.render_rows()
        elif self.mode == "failed":
            if self.entry in self.closed and not self.base.query(GoneView):
                self.grace_until = time.monotonic() + GRACE_SECONDS
                self._mount(GoneView(f"Q{self.entry[1]}", self.closed[self.entry]))
        elif self.mode == "group":
            self._poll_group({q.id for q in open_qs})
        elif self.mode == "remote" and not self.base.query(GoneView):
            m, data = self.remote_open
            found = remote.group_by_key(self.store, self.entry[1])
            if not found or all(q["id"] != data["id"] for q in found[1]):
                # answered, withdrawn or dismissed over there: an inert placeholder, like a
                # closed local tab (Enter does nothing; Esc goes back to the list)
                self.grace_until = time.monotonic() + GRACE_SECONDS
                self._mount(GoneView(f"{m.get('label')}:Q{data['id']}", "closed on the remote machine"))

    def _entry_reason(self, entry: Entry) -> str:
        if entry[0] == "remote":
            return "answered or closed"
        if entry[0] == "routbox":
            return "sent"
        if entry[0] == "rproblem":
            return "delivered or cleared"
        if entry[0] == "failed":
            return self._gone_reason("failed", entry[1])
        qs = self.store.group(entry[1], include_closed=True)
        reasons = {self._gone_reason("q", q.id) for q in qs}
        return reasons.pop() if len(reasons) == 1 else "closed"

    def _poll_group(self, open_ids: set[int]) -> None:
        ended = [t for t in self.tabs if t[1] not in open_ids and t not in self.gone]
        self.tabs = [t for t in self.tabs if t[1] in open_ids or t == self.current or t in self.gone]
        if self.current in ended:
            self.gone[self.current] = self._gone_reason(*self.current)
            self.notify(f"Q{self.current[1]} was closed ({self.gone[self.current]})", severity="warning")
            self.grace_until = time.monotonic() + GRACE_SECONDS
            self.show(self.current)  # the same tab, now the placeholder
            return
        view = self.base.query("#body > *").first() if self.base.query("#body > *") else None
        if isinstance(view, QuestionView):
            view.set_mode(not self.single, self._last_open(view.q.id))
        elif isinstance(view, ReviewView):
            view.qids = self.open_qids
            view.reload()
        self.render_chips()

    # -- operator actions -------------------------------------------------------------

    def navigate(self, target: tuple[str, int] | int | None) -> None:
        """Move between the open group's tabs; closed placeholders drop out on this move."""
        tabs = self.all_tabs
        if isinstance(target, int):
            i = tabs.index(self.current) if self.current in tabs else 0
            target = tabs[(i + target) % len(tabs)] if tabs else None
        self.tabs = [t for t in self.tabs if t not in self.gone or t == target]
        self.gone = {t: r for t, r in self.gone.items() if t == target}
        tabs = self.all_tabs
        if target not in tabs:
            target = tabs[0] if tabs else None
        self.show(target)

    def action_switch(self, delta: int) -> None:
        if self.mode == "remote":
            return  # one remote question at a time: no tabs
        if self.held:  # Tab/→ (the switch keys): release a deep link, nothing more
            if delta > 0 and time.monotonic() - self.held_at >= RELEASE_GRACE:
                self.held = False
                self.remove_class("held")
            return
        if self.mode == "empty":  # Tab/→ from the empty view: the list of what arrived
            self.show_list(self.entries[0] if self.entries else None)
        else:
            self.navigate(delta)

    def back_to_list(self) -> None:
        """After the operator's send, hide or cancel: the list, with the cursor still on the
        entry just handled (an inert placeholder if it ended); close if nothing is left."""
        self.direct = False
        if not self.live_entries:
            self.call_next(self.action_quit)
            return
        self.show_list(self.list_cursor)

    async def action_back(self) -> None:
        """Esc: from a group or ⚠ entry back to the list (or, when the popup opened straight
        into the only group, close it); from the list, close the popup. Never sends."""
        if self.mode in ("group", "failed", "remote") and not self.direct:
            self.show_list(self.list_cursor)
            return
        await self.action_quit()

    def on_unmount(self) -> None:
        self._unmounted = True
        service.drain(1.0)  # tab labels/markers queued by this popup's last actions (bounded)
        timer = getattr(self, "_poll_timer", None)
        if timer is not None:
            timer.stop()  # no poll may land on a screen being torn down
        lock = getattr(self, "_popup_lock", None)
        if lock is not None:
            lock.close()

    def on_key(self, event) -> None:
        if self.mode == "empty" and len(self.screen_stack) == 1:  # empty view: any key closes,
            if event.key in NAV_KEYS and self.entries:           # except Tab/arrows to arrivals
                return
            event.stop()
            self.exit()

    def on_group_list_view_open(self, event: GroupListView.Open) -> None:
        self.open_group(event.entry)

    @trace.traced("popup.answered")
    @guarded
    def on_question_view_answered(self, event: QuestionView.Answered) -> None:
        if self.mode == "remote":
            self._answer_remote(event)
            return
        if self.mode != "group" or ("q", event.qid) in self.gone or self.current != ("q", event.qid):
            return  # never act for a question that is not the one on screen
        if self.single:  # a lone question the operator opened: answer and send at once
            d = event.detail or {}
            self._sent(self.entry, whole=True)
            _send(self, self.store, [event.qid],
                  lambda s, deliver: [x for x in [service.answer(s, event.qid, d.get("text"), choices=d.get("choices"),
                                                                  notes=d.get("notes"), deliver=deliver)] if x])
            return
        if not self.store.set_draft(event.qid, event.detail):
            self.notify(f"Q{event.qid} is no longer open", severity="warning")
        if event.detail is None:
            self.render_chips()
            return
        self.navigate(1)  # Claude Code moves to the next tab (the last one: Submit)

    def _answer_remote(self, event: QuestionView.Answered) -> None:
        """A remote question: queue the answer for the worker to send (local DB write only)."""
        m, data = self.remote_open
        if event.qid != data["id"] or event.detail is None:
            return
        try:
            remote.enqueue(self.store, m["machine_id"], data, "answer", event.detail)
        except (LookupError, sqlite3.Error) as exc:
            self.notify(f"not queued: {exc}", severity="warning")
            return
        label = m.get("label") or m["machine_id"]
        self.notify(f"{label}:Q{data['id']} answered · sending to {label}")
        found = remote.group_by_key(self.store, self.entry[1])
        if not found:
            self.closed[self.entry] = f"answered · sending to {label}"
        self.back_to_list()

    def _sent(self, entry: Entry | None, whole: bool) -> None:
        """After the operator's send: back to the list now (not when the send finishes). A
        group sent in full stays as an inert row; a partly sent one stays openable."""
        if entry is not None and whole:
            self.closed[entry] = "sending…"
        self.back_to_list()

    def on_question_view_chat(self, event: QuestionView.Chat) -> None:
        self._focus_pane(event.q)

    def on_review_view_edit(self, event: ReviewView.Edit) -> None:
        self.navigate(("q", event.qid))

    @trace.traced("popup.submit")
    def on_review_view_submit(self, event: ReviewView.Submit) -> None:
        whole = set(event.qids) >= set(self.open_qids)
        entry = self.entry
        _send(self, self.store, event.qids, lambda s, deliver: service.submit(s, event.qids, deliver=deliver))
        self._sent(entry, whole)

    @trace.traced("popup.retry")
    def on_failed_view_retry(self, event: FailedView.Retry) -> None:
        def retry(s: Store, deliver: bool) -> list[Question]:
            q = s.get(event.qid)
            err = (q.delivery_error or "") if q else ""
            if err.startswith("sending:") and err > f"sending:{store_now(-CLAIM_STALE)}":
                raise AlreadySending(event.qid)  # its sender is at it right now
            if deliver:
                service.deliver(s, q, force=True)  # the operator's explicit retry / "Send now"
            return [s.get(event.qid)]

        entry = self.entry
        _send(self, self.store, [event.qid], retry, force=True)
        self._sent(entry, whole=True)

    @guarded
    def on_failed_view_hide(self, event: FailedView.Hide) -> None:
        self.store.hide_failed(event.qid)
        self.closed[("failed", event.qid)] = "hidden"
        self.back_to_list()

    @trace.traced("popup.focus_pane")
    def _focus_pane(self, q: Question | None) -> None:
        if q is None:
            return
        if not q.pane_id:
            self.notify("could not focus: no pane", severity="warning")
            return
        pane = q.pane_id

        def run() -> None:  # a herdr call: off the UI thread
            err = herdr.focus_agent(pane)
            try:
                self.call_from_thread(self._focused, pane, err)
            except (RuntimeError, concurrent.futures.CancelledError):
                if self.is_running:
                    raise

        self.run_worker(run, thread=True, group="focus")

    def _focused(self, pane: str, err: str | None) -> None:
        if err:
            self.notify(f"could not focus {pane}: {err}", severity="warning")
        else:
            self.exit()  # the question stays open; discuss it in the agent's chat

    def _entry_question(self, entry: Entry) -> Question | None:
        if entry[0] == "failed":
            return self.store.get(entry[1])
        qs = self.store.group(entry[1])
        return qs[0] if qs else None

    def action_goto(self) -> None:
        cur = self.entry if self.mode == "remote" else (self._list_view().cursor if self.mode == "list" and self._list_view() else None)
        if cur and cur[0] in ("remote", "routbox"):
            self.notify("That agent runs on a remote machine: open it from herdr's machine view")
            return
        if self.mode == "list":
            view = self._list_view()
            if view and view.cursor:
                self._focus_pane(self._entry_question(view.cursor))
        elif self.mode == "failed" and self.entry:
            self._focus_pane(self.store.get(self.entry[1]))
        elif self.current and self.current != SUBMIT:
            self._focus_pane(self.store.get(self.current[1]))

    def action_dismiss(self) -> None:
        if self.mode == "list":
            view = self._list_view()
            entry = view.cursor if view else None
            if entry and entry[0] == "routbox" and entry not in self.closed:
                row = next((r for r in remote.outbox_rows(self.store) if r["request_id"] == entry[1]), None)
                if not remote.clearable(self.store, entry[1]):
                    return  # still queued, or being sent right now: nothing to clear
                expired = row and row["state"] == "queued"
                if expired:
                    remote._settle(self.store, entry[1], "failed", "expired")
                remote.mark_seen(self.store, entry[1])  # a failed/rejected/expired action: just clear it
                self.closed[entry] = "cleared"
                view.render_rows()
                return
            if entry and entry[0] == "rproblem" and entry not in self.closed:
                mid, _, rid = str(entry[1]).partition("|")
                remote.clear_problem(self.store, mid, int(rid))  # the remote keeps it for check_answers
                self.closed[entry] = "cleared"
                view.render_rows()
                return
            if entry and entry[0] == "remote" and entry not in self.closed:
                found = remote.group_by_key(self.store, entry[1])
                if not found:
                    return
                m, qs = found
                label = m.get("label") or m["machine_id"]

                @guarded
                def done_remote(confirmed: bool | None) -> None:
                    if confirmed:
                        for data in qs:
                            remote.enqueue(self.store, m["machine_id"], data, "dismiss")
                        self.closed[entry] = f"dismissing on {label}"
                        lv = self._list_view()
                        if lv:
                            lv.render_rows()

                self.push_screen(ConfirmDismiss(f"{label}:Q{qs[0]['id']}", service.first_line(qs[0]["question"], 70)),
                                 done_remote)
                return
            if entry and entry[0] == "failed" and entry not in self.closed and _gone(self.store.get(entry[1])):
                # an agent-gone row: hiding it loses nothing (check_answers still has it)
                self.store.hide_failed(entry[1])
                self.closed[entry] = "hidden"
                view.render_rows()
                return
            if not entry or entry[0] != "group" or entry in self.closed:
                return
            qs = [q for q in self.store.group(entry[1]) if q.status == "open"]
            if not qs:
                return
            what = qs[0].ref if len(qs) == 1 else f"{qs[0].ref}–{qs[-1].ref} ({len(qs)} questions)"

            @guarded
            @trace.traced("popup.dismiss_group")
            def done_group(confirmed: bool | None) -> None:
                if confirmed:
                    closed = [q.ref for q in qs if service.close(self.store, q.id, "dismissed")]
                    if closed:
                        self.notify(f"{', '.join(closed)} dismissed")
                    self.closed[entry] = "dismissed"
                    lv = self._list_view()
                    if lv:
                        lv.render_rows()

            self.push_screen(ConfirmDismiss(what, service.first_line(qs[0].question, 70)), done_group)
            return
        if self.mode != "group" or not self.current or self.current[0] != "q" or self.current in self.gone:
            return
        q = self.store.get(self.current[1])
        if q is None:
            return

        @guarded
        @trace.traced("popup.dismiss")
        def done(confirmed: bool | None) -> None:
            if confirmed and service.close(self.store, q.id, "dismissed"):
                self.notify(f"{q.ref} dismissed")
                self.gone[("q", q.id)] = "dismissed"
                self.navigate(1)

        self.push_screen(ConfirmDismiss(q.ref, service.first_line(q.question, 70)), done)

    async def action_quit(self) -> None:
        # let an in-flight send finish before the popup (and its process) goes away
        with trace.step("popup.quit_wait"):
            await self.workers.wait_for_complete()
        trace.record("popup.exit", 0)
        self.exit()
