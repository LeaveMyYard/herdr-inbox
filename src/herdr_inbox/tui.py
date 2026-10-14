"""Textual inbox, opened by herdr as a popup (``inbox tui``).

The popup opens straight into a tabbed view modelled on Claude Code's own
AskUserQuestion prompt (strings and behaviour checked against the Claude Code
2.1.299 binary). Every open question is a tab: first those from the pane that
was focused when the popup opened, then the rest oldest first, with the
questions an agent filed together kept adjacent; a dim tab label separates
agents. New questions are appended while the popup is open.

- one open question in total: Enter on an option, or text in "Type
  something.", answers and sends at once;
- several: answering drafts it (kept in the database, so closing the popup
  loses nothing) and moves to the next tab; the last tab is Submit: "Review
  your answers", "You have not answered all questions" when some are blank,
  "Ready to submit your answers?" (Submit answers / Cancel). Submitting sends
  every answered question at once, batched per pane; the rest stay open;
- "Type something." is Claude Code's Other (the answer in single-select, added
  to the picks in multi-select); "Chat about this" (single-select) focuses the
  asking agent's pane and leaves the question open; ``n`` adds notes; ``g``
  goes to the asking agent's pane; ``d`` dismisses the current question after
  a one-key confirmation; Esc closes the popup without sending;
- a poll never replaces the current view with another question: if its own
  question closes, the tab becomes an inert placeholder until the operator
  switches tabs; failed deliveries show up as ``⚠ Q<n> not delivered`` tabs;
- options may carry a ``preview``: options left, the highlighted option's
  preview right ("No preview available" otherwise).
"""

from __future__ import annotations

import functools
import os
import sqlite3

from rich.markup import escape
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, HorizontalScroll, Vertical, VerticalScroll
from textual.message import Message
from textual.screen import ModalScreen, Screen
from textual.widgets import Input, Static

from . import herdr, service
from .store import Question, Store

POLL_SECONDS = 2.0
OTHER = "Type something."
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


def _draft_line(q: Question) -> str:
    d = q.draft_json or {}
    return service.format_answer(d.get("choices") or [], d.get("notes"), d.get("text"))


def _typing(screen: Screen) -> bool:
    return isinstance(screen.focused, Input)


def _cursor(here: bool) -> str:
    # one fixed cell on every row, its own span (exported screenshots drop a span's leading spaces)
    return "[$accent]❯[/]" if here else "[$background] [/]"


def _close(app: App) -> None:
    """Close the popup without sending (drafts stay); waits for an in-flight send."""
    app.call_next(app.action_quit)


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
        meta = f"{q.ref} · {q.tab_label or q.pane_id or 'no pane'}" + (f" · {q.asked_by}" if q.asked_by else "")
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
            return f"{cur} [$text-muted]  [/] " + (f"[b $accent]{label}[/]" if here else f"[$foreground]{label}[/]")
        if kind == "chat":
            return f"{cur} {number} " + (f"[b $accent]{CHAT}[/]" if here else f"[$foreground]{CHAT}[/]")
        box = ""
        if self.q.multi_select:
            on = (opt_i in self.picked) if kind == "opt" else bool(self.text)
            box = "[x] " if on else "[ ] "
        if kind == "other":
            if self.text:
                label = f"[i]{escape(box + self.text)}[/i]"
            else:
                label = f"[dim]{escape(box)}{OTHER}[/dim]"
            if here:
                label = f"[b $accent]{escape(box + (self.text or OTHER))}[/]"
            return f"{cur} {number} {label}"
        opt = self.options[opt_i]
        text = escape(box + opt["label"])
        lines = [f"{cur} {number} " + (f"[b $accent]{text}[/]" if here else f"[$foreground]{text}[/]")]
        if opt.get("description"):
            lines.append(f"      [dim]{escape(opt['description'])}[/]")
        if self.notes.get(opt_i):
            lines.append(f"      [i]Notes: {escape(self.notes[opt_i])}[/]")
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
        return action == "back" or not _typing(self.screen)

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
        self.cursor: int | None = None  # starts on "Submit answers"

    def compose(self) -> ComposeResult:
        yield VerticalScroll(Static(id="review-body"), classes="choices")
        yield Static(classes="hint")

    def on_mount(self) -> None:
        self.reload()

    @guarded
    def reload(self) -> None:
        self.items = [q for q in (self.store.get(i) for i in self.qids) if q and q.status == "open"]
        n = len(self.items)
        self.cursor = n if self.cursor is None else min(self.cursor, n + 1)
        self.render_rows()

    @property
    def drafted(self) -> list[Question]:
        return [q for q in self.items if q.draft_json]

    def render_rows(self) -> None:
        lines = ["[b]Review your answers[/b]", ""]
        for i, q in enumerate(self.items):
            mark = "[$accent]❯[/]" if i == self.cursor else "[$foreground]●[/]"
            where = f"{q.ref} · {escape(q.tab_label or q.pane_id or '-')} · "
            lines.append(f"{mark} [$foreground]{where}{escape(service.first_line(q.question, 80))}[/]")
            if q.draft_json:
                lines.append(f"  [$accent]→[/] [b]{escape(_draft_line(q))}[/b]")
            else:
                lines.append("  [dim]→ (not answered)[/dim]")
        if len(self.drafted) < len(self.items):
            lines += ["", "[$warning]⚠ You have not answered all questions[/]"]
        lines += ["", "[b]Ready to submit your answers?[/b]", ""]
        n = len(self.items)
        for k, label in enumerate(("Submit answers", "Cancel")):
            here = self.cursor == n + k
            text = f"[b $accent]{label}[/]" if here else f"[$foreground]{label}[/]"
            lines.append(f"{_cursor(here)} [$text-muted]{k + 1}.[/] {text}")
        self.query_one("#review-body", Static).update("\n".join(lines))
        self.query_one(".hint", Static).update(
            "Enter to select · ↑/↓ to navigate · Tab/Arrow keys to switch questions · Esc to cancel")

    def action_move(self, delta: int) -> None:
        self.cursor = (self.cursor + delta) % (len(self.items) + 2)
        self.render_rows()

    def action_pick(self, k: int) -> None:
        self.cursor = len(self.items) + k
        self.action_select()

    def action_select(self) -> None:
        n = len(self.items)
        if self.cursor < n:
            self.post_message(self.Edit(self.items[self.cursor].id))
        elif self.cursor == n:
            if self.drafted:
                self.post_message(self.Submit([q.id for q in self.drafted]))
            else:
                self.notify("Nothing answered yet", severity="warning")
        else:
            _close(self.app)

    @guarded
    def action_drop(self) -> None:
        if self.cursor < len(self.items):
            self.store.set_draft(self.items[self.cursor].id, None)
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
.preview-box { width: 1fr; border-left: solid $accent; padding: 0 1; }
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
        yield Static(f"[b]{self.ref} was closed[/b] [dim]({escape(self.reason)})[/dim]", classes="question")
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
        meta = f"{q.ref} · {q.tab_label or q.pane_id or 'no pane'}" + (f" · {q.asked_by}" if q.asked_by else "")
        yield Static(f"[$warning]⚠ {q.ref} not delivered[/]: [dim]{escape(q.delivery_error or '')}[/dim]",
                     classes="question")
        yield Static(escape(meta), classes="meta")
        yield Static(f"[b]{escape(q.question)}[/b]\n  [$accent]→[/] {escape(q.answer or '')}", classes="question")
        yield Static(id="failed-rows")
        yield Static("Enter to select · Tab/Arrow keys to navigate · g go to agent · Esc to cancel", classes="hint")

    def on_mount(self) -> None:
        self.render_rows()

    def render_rows(self) -> None:
        labels = ("Retry delivery", "Stop showing it here (the agent can still fetch it with check_answers)")
        lines = []
        for k, label in enumerate(labels):
            here = k == self.cursor
            text = f"[b $accent]{label}[/]" if here else f"[$foreground]{label}[/]"
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
    """One-key confirmation: d (or y) dismisses, any other key keeps the question."""

    CSS = """
    ConfirmDismiss { align: center middle; }
    #confirm { width: auto; max-width: 80%; padding: 1 2; border: round $warning; background: $panel; }
    """

    def __init__(self, q: Question):
        super().__init__()
        self.q = q

    def compose(self) -> ComposeResult:
        yield Static(f"Dismiss [b]{self.q.ref}[/b] without answering?\n[dim]{escape(service.first_line(self.q.question, 70))}"
                     f"[/dim]\n\n[$warning]d[/] to dismiss · any other key to keep it", id="confirm")

    def on_key(self, event) -> None:
        event.stop()
        self.dismiss(event.key in ("d", "y"))


def _after_submit(app: "InboxApp", done: list[Question], wanted: int) -> None:
    failed = [q for q in done if not q.delivered_at]
    if not done:
        app.notify("Nothing sent: those questions are no longer open", severity="warning")
    elif failed:
        app.notify(f"Sent {', '.join(q.ref for q in done)}; not delivered: "
                   f"{', '.join(f'{q.ref} ({q.delivery_error})' for q in failed)}", severity="warning")
    else:
        app.notify(f"Sent {', '.join(q.ref for q in done)}")
    if done and len(done) < wanted:
        app.notify(f"{wanted - len(done)} answered question(s) had closed meanwhile", severity="warning")
    app.poll()
    if not app.tabs and done and not failed:
        app.exit()  # like Claude Code's prompt: gone once everything is answered


def _send(app: "InboxApp", store: Store, qids: list[int], work) -> None:
    """Send in a background thread: delivery talks to herdr (and may wait for the
    tab-label lock), and the popup must never freeze on it."""
    app.notify(f"Sending {', '.join(f'Q{q}' for q in qids)}…")

    def run() -> None:
        try:
            with Store(store.path) as own:  # sqlite connections stay in the thread that made them
                done = work(own)
        except sqlite3.OperationalError:
            app.call_from_thread(app.notify, BUSY, severity="warning")
            return
        app.call_from_thread(_after_submit, app, done, len(qids))

    app.run_worker(run, thread=True, group="send")


def tab_order(questions: list[Question], focus_pane: str | None) -> list[int]:
    """Questions from the focused pane first, then the rest oldest first; a group stays together."""
    groups: dict[int, list[Question]] = {}
    for q in sorted(questions, key=lambda q: q.id):
        groups.setdefault(q.group, []).append(q)
    ordered = sorted(groups.values(), key=lambda g: (g[0].pane_id != focus_pane, g[0].id))
    return [q.id for g in ordered for q in g]


SUBMIT = ("submit", 0)


class InboxApp(App):
    """Every open question as a tab, plus Submit when there are several, plus a ⚠ tab per
    answer that was not delivered.

    Tabs are ("q", id) or ("failed", id). Polls only ever add tabs at the end or drop tabs
    that are not current; the current tab's view is never replaced by another question's.
    Whether Enter sends at once (exactly one open question) is decided when the popup opens
    and re-decided only when the operator navigates.
    """

    TITLE = "herdr inbox"
    CSS = """
    Screen { padding: 1 2; }
    #chips { height: 1; margin-bottom: 1; scrollbar-size-horizontal: 0; }
    #chips > Static { width: auto; margin-right: 2; }
    #chips > .sep { color: $text-muted; }
    #chips > .warn { color: $warning; }
    #empty { display: none; }
    InboxApp.empty #empty { display: block; }
    InboxApp.empty #chips, InboxApp.empty #body { display: none; }
    """ + VIEW_CSS
    BINDINGS = [
        Binding("left", "switch(-1)", show=False),
        Binding("shift+tab", "switch(-1)", show=False, priority=True),
        Binding("right", "switch(1)", show=False),
        Binding("tab", "switch(1)", show=False, priority=True),
        Binding("d", "dismiss", show=False),
        Binding("g", "goto", show=False),
        Binding("escape", "quit", show=False),
    ]

    def __init__(self, store: Store | None = None, focus_pane: str | None = None):
        super().__init__()
        self.store = store or Store()
        self.focus_pane = focus_pane
        self.tabs: list[tuple[str, int]] = []
        self.current: tuple[str, int] | None = None
        self.gone: dict[tuple[str, int], str] = {}  # tabs whose question closed while shown
        self.single = False  # Enter sends at once; decided on open and on navigation only

    @property
    def base(self) -> Screen:
        return self.screen_stack[0]

    # -- state -------------------------------------------------------------------

    def _live(self) -> tuple[list[Question], list[Question]]:
        return self.store.list(), self.store.failed_deliveries()

    @property
    def open_qids(self) -> list[int]:
        return [i for k, i in self.tabs if k == "q" and (k, i) not in self.gone]

    @property
    def all_tabs(self) -> list[tuple[str, int]]:
        return self.tabs + ([] if self.single or not self.open_qids else [SUBMIT])

    def _decide_mode(self) -> None:
        self.single = len(self.open_qids) == 1

    def compose(self) -> ComposeResult:
        yield HorizontalScroll(id="chips")
        yield Vertical(id="body")
        yield Static("No open questions", id="empty")

    def on_mount(self) -> None:
        if self.focus_pane is None:
            self.focus_pane = (herdr.invocation_pane() or os.environ.get("INBOX_FOCUS_PANE")
                               or herdr.focused_pane())
        try:
            open_qs, failed = self._live()
        except sqlite3.OperationalError:
            open_qs, failed = [], []
            self.notify(BUSY, severity="warning")
        self.tabs = [("q", i) for i in tab_order(open_qs, self.focus_pane)] + [("failed", q.id) for q in failed]
        self._decide_mode()
        self.show(self.all_tabs[0] if self.all_tabs else None)
        self.set_interval(POLL_SECONDS, self.poll)

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        if action in ("switch", "dismiss", "goto"):
            return bool(self.tabs) and not isinstance(self.focused, Input)
        return True

    # -- rendering -----------------------------------------------------------------

    def show(self, tab: tuple[str, int] | None) -> None:
        """Mount the view for ``tab`` (only ever called on open or for an operator action)."""
        self.current = tab
        self.set_class(tab is None, "empty")
        body = self.base.query_one("#body")
        body.remove_children()
        if tab is None:
            self.base.query_one("#chips").remove_children()
            return
        kind, qid = tab
        if tab == SUBMIT:
            view = ReviewView(self.store, self.open_qids)
        elif tab in self.gone:
            view = GoneView(f"Q{qid}", self.gone[tab])
        elif kind == "failed":
            view = FailedView(self.store.get(qid))
        else:
            view = QuestionView(self.store.get(qid), in_group=not self.single, last=self._last_open(qid))
        body.mount(view)
        view.focus()
        self.render_chips()

    def _last_open(self, qid: int) -> bool:
        return self.single or (self.open_qids[-1:] == [qid])

    def render_chips(self) -> None:
        row = self.base.query_one("#chips", HorizontalScroll)
        row.remove_children()
        tabs = self.all_tabs
        if len(tabs) <= 1:
            return
        chips, current, last_tab = [], None, object()
        for tab in tabs:
            here = tab == self.current
            if tab == SUBMIT:
                text, classes = "Submit", ""
            else:
                kind, qid = tab
                q = self.store.get(qid)
                where = (q.tab_label or q.pane_id) if q else None
                if where != last_tab:  # agent boundary
                    chips.append(Static(f"│ {escape(where or 'no pane')}", classes="sep"))
                    last_tab = where
                if tab in self.gone:
                    text, classes = f"✕ Q{qid}", "sep"
                elif kind == "failed":
                    text, classes = f"⚠ Q{qid} not delivered", "warn"
                else:
                    text, classes = f"{'✓' if q and q.draft_json else '●'} {escape(q.header or q.ref)}", ""
            chip = Static(f"[reverse b] {text} [/]" if here else text,
                          classes=" ".join(c for c in (classes, "current" if here else "") if c))
            chips.append(chip)
            if here:
                current = chip
        row.mount_all(chips)
        if current is not None:
            self.call_after_refresh(current.scroll_visible, animate=False)

    def _gone_reason(self, kind: str, qid: int) -> str:
        q = self.store.get(qid)
        if kind == "failed":
            return "delivered" if q and q.delivered_at else "no longer pending"
        if q is None:
            return "removed"
        return {"withdrawn": "withdrawn by the agent", "answered": "answered elsewhere",
                "dismissed": "dismissed"}.get(q.status, q.status)

    @guarded
    def poll(self) -> None:
        """New questions and failed deliveries are appended; tabs that ended are dropped,
        except the current one, which turns into an inert placeholder. No view is rebuilt
        unless the current tab itself ended, and the send mode does not change here."""
        if len(self.screen_stack) > 1:  # a confirmation is open
            return
        open_qs, failed = self._live()
        live = {("q", q.id) for q in open_qs} | {("failed", q.id) for q in failed}
        known = set(self.tabs)
        ended = [t for t in self.tabs if t not in live and t not in self.gone]
        self.tabs = [t for t in self.tabs if t in live or t == self.current]
        n_q = max((i for i, t in enumerate(self.tabs) if t[0] == "q"), default=-1) + 1
        new_q = [("q", i) for i in tab_order([q for q in open_qs if ("q", q.id) not in known], None)]
        self.tabs[n_q:n_q] = new_q
        self.tabs += [("failed", q.id) for q in failed if ("failed", q.id) not in known]
        if self.current in ended:
            self.gone[self.current] = self._gone_reason(*self.current)
            self.notify(f"Q{self.current[1]} was closed ({self.gone[self.current]})", severity="warning")
            self.show(self.current)  # the same tab, now the placeholder
            return
        if self.current is None and self.tabs:  # was empty; something arrived
            self._decide_mode()
            self.show(self.all_tabs[0])
            return
        view = self.base.query("#body > *").first() if self.base.query("#body > *") else None
        if isinstance(view, QuestionView):
            view.set_mode(not self.single, self._last_open(view.q.id))
        elif isinstance(view, ReviewView):
            view.qids = self.open_qids
            view.reload()
        self.render_chips()

    # -- navigation (operator actions) -------------------------------------------------

    def navigate(self, target: tuple[str, int] | int | None) -> None:
        """Move to ``target`` (a tab, or a relative step): placeholders are dropped and the
        send mode is re-decided here, on the operator's move."""
        tabs = self.all_tabs
        if isinstance(target, int):
            i = tabs.index(self.current) if self.current in tabs else 0
            target = tabs[(i + target) % len(tabs)] if tabs else None
        self.tabs = [t for t in self.tabs if t not in self.gone or t == target]
        self.gone = {t: r for t, r in self.gone.items() if t == target}
        self._decide_mode()
        tabs = self.all_tabs
        if target not in tabs:
            target = tabs[0] if tabs else None
        self.show(target)

    def action_switch(self, delta: int) -> None:
        self.navigate(delta)

    def on_key(self, event) -> None:
        if self.current is None and len(self.screen_stack) == 1:  # empty state: any key closes
            event.stop()
            self.exit()

    @guarded
    def on_question_view_answered(self, event: QuestionView.Answered) -> None:
        if ("q", event.qid) in self.gone or self.current != ("q", event.qid):
            return  # never act for a question that is not the one on screen
        if self.single:  # the only open question: answer and send at once, like Claude Code
            d = event.detail or {}
            _send(self, self.store, [event.qid],
                  lambda s: [x for x in [service.answer(s, event.qid, d.get("text"), choices=d.get("choices"),
                                                         notes=d.get("notes"))] if x])
            return
        if not self.store.set_draft(event.qid, event.detail):
            self.notify(f"Q{event.qid} is no longer open", severity="warning")
        if event.detail is None:
            self.render_chips()
            return
        self.navigate(1)  # Claude Code moves to the next tab (the last one: Submit)

    def on_question_view_chat(self, event: QuestionView.Chat) -> None:
        self._focus_pane(event.q)

    def on_review_view_edit(self, event: ReviewView.Edit) -> None:
        self.navigate(("q", event.qid))

    def on_review_view_submit(self, event: ReviewView.Submit) -> None:
        _send(self, self.store, event.qids, lambda s: service.submit(s, event.qids))

    def on_failed_view_retry(self, event: FailedView.Retry) -> None:
        def retry(s: Store) -> list[Question]:
            q = s.get(event.qid)
            service.deliver(s, q)
            return [s.get(event.qid)]

        _send(self, self.store, [event.qid], retry)

    @guarded
    def on_failed_view_hide(self, event: FailedView.Hide) -> None:
        self.store.hide_failed(event.qid)
        self.gone[("failed", event.qid)] = "hidden"
        self.navigate(1)

    def _focus_pane(self, q: Question | None) -> None:
        if q is None:
            return
        err = herdr.focus_agent(q.pane_id) if q.pane_id else "no pane"
        if err:
            self.notify(f"could not focus {q.pane_id}: {err}", severity="warning")
        else:
            self.exit()  # the question stays open; discuss it in the agent's chat

    def action_goto(self) -> None:
        if self.current and self.current != SUBMIT:
            self._focus_pane(self.store.get(self.current[1]))

    def action_dismiss(self) -> None:
        if not self.current or self.current[0] != "q" or self.current in self.gone:
            return
        q = self.store.get(self.current[1])
        if q is None:
            return

        @guarded
        def done(confirmed: bool | None) -> None:
            if confirmed and service.close(self.store, q.id, "dismissed"):
                self.notify(f"{q.ref} dismissed")
                self.gone[("q", q.id)] = "dismissed"
                self.navigate(1)

        self.push_screen(ConfirmDismiss(q), done)

    async def action_quit(self) -> None:
        # let an in-flight send finish before the popup (and its process) goes away
        await self.workers.wait_for_complete()
        self.exit()
