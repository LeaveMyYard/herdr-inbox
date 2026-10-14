"""Textual inbox, opened by herdr as a popup (``inbox tui``).

The question UI mimics Claude Code's own AskUserQuestion prompt (strings and
behaviour checked against the Claude Code 2.1.299 binary):

- one question: Enter on an option, or text in "Type something.", answers and
  sends at once;
- a group: one tab per question across the top plus a final Submit tab;
  answering a question moves to the next tab; the Submit tab shows "Review
  your answers", warns "You have not answered all questions" when some are
  blank, and asks "Ready to submit your answers?" (Submit answers / Cancel).
  Our additions: answers in a group are drafts kept in the database (closing
  the popup loses nothing), and submitting with blanks sends only the answered
  ones and leaves the rest open;
- "Type something." is Claude Code's Other: in single-select it is the answer,
  in multi-select it is added to the picks; "Chat about this" (single-select)
  focuses the asking agent's pane and leaves the question open; ``n`` adds
  notes to an option; Esc goes back without sending;
- options may carry a ``preview``: then the options sit left and the
  highlighted option's preview right ("No preview available" otherwise).
"""

from __future__ import annotations

import functools
import sqlite3

from rich.markup import escape
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.message import Message
from textual.screen import Screen
from textual.widgets import DataTable, Footer, Input, Static

from . import herdr, service
from .cli import _age
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


def _state(q: Question) -> str:
    if q.status == "open":
        return "drafted" if q.draft_json else "open"
    if q.status != "answered":
        return q.status
    if q.delivered_at:
        return "answered ✓"
    return "answered !" if q.delivery_error else "answered"


def _draft_line(q: Question) -> str:
    d = q.draft_json or {}
    return service.format_answer(d.get("choices") or [], d.get("notes"), d.get("text"))


def _refs(qs: list[Question]) -> str:
    return qs[0].ref if len(qs) == 1 else f"{qs[0].ref}–{qs[-1].ref}"


def _typing(screen: Screen) -> bool:
    return isinstance(screen.focused, Input)


def _cursor(here: bool) -> str:
    # one fixed cell on every row, its own span (exported screenshots drop a span's leading spaces)
    return "[$accent]❯[/]" if here else "[$background] [/]"


def _back(app: App) -> None:
    """Leave the current view without sending; the list reflects drafts right away."""
    app.pop_screen()
    if isinstance(app, InboxApp) and app.screen is app.base:
        app.refresh_list()


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
        return f"{pick} · {nav} · press n to add notes · Esc to cancel"

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
            _back(self.app)

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

    def __init__(self, store: Store, qids: list[int] | None = None):
        """``qids``: one group's questions; None means every draft (the global review)."""
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
        if self.qids is None:
            self.items = self.store.drafted()
        else:
            self.items = [q for q in (self.store.get(i) for i in self.qids) if q and q.status == "open"]
        n = len(self.items)
        self.cursor = n if self.cursor is None else min(self.cursor, n + 1)
        self.render_rows()

    @property
    def drafted(self) -> list[Question]:
        return [q for q in self.items if q.draft_json]

    def render_rows(self) -> None:
        title = "Review your answers" + ("" if self.qids is not None else " (all drafts)")
        lines = [f"[b]{title}[/b]", ""]
        for i, q in enumerate(self.items):
            mark = "[$accent]❯[/]" if i == self.cursor else "[$foreground]●[/]"
            where = "" if self.qids is not None else f"{q.ref} · {escape(q.tab_label or q.pane_id or '-')} · "
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
            _back(self.app)

    @guarded
    def action_drop(self) -> None:
        if self.cursor < len(self.items):
            self.store.set_draft(self.items[self.cursor].id, None)
            self.reload()

    def action_back(self) -> None:
        _back(self.app)


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


def _after_submit(app: "InboxApp", done: list[Question], wanted: int) -> None:
    if not done:
        app.notify("Nothing sent: those questions are no longer open", severity="warning")
    else:
        sent = [q for q in done if q.delivered_at]
        msg = f"Sent {', '.join(q.ref for q in done)}"
        if len(sent) < len(done):
            failed = [q for q in done if not q.delivered_at]
            msg += f"; not delivered: {', '.join(f'{q.ref} ({q.delivery_error})' for q in failed)}"
            app.notify(msg, severity="warning")
        else:
            app.notify(msg)
        if len(done) < wanted:
            app.notify(f"{wanted - len(done)} answered question(s) had closed meanwhile", severity="warning")
    app.refresh_list()


def _send(app: "InboxApp", store: Store, qids: list[int], work) -> None:
    """Leave the view and send in a background thread: delivery talks to herdr (and may wait
    for the tab-label lock), and the popup must never freeze on it."""
    app.pop_screen()
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


class GroupScreen(Screen):
    """One question alone, or a group as tabs (✓ answered · ● not yet) plus a Submit tab."""

    CSS = """
    GroupScreen { padding: 1 2; }
    #chips { height: 1; margin-bottom: 1; }
    """ + VIEW_CSS
    BINDINGS = [
        Binding("left", "switch(-1)", show=False),
        Binding("shift+tab", "switch(-1)", show=False, priority=True),
        Binding("right", "switch(1)", show=False),
        Binding("tab", "switch(1)", show=False, priority=True),
    ]

    def __init__(self, store: Store, qids: list[int], start: int = 0):
        super().__init__()
        self.store = store
        self.qids = qids
        self.index = start

    @property
    def grouped(self) -> bool:
        return len(self.qids) > 1

    @property
    def review_index(self) -> int:
        return len(self.qids)

    def compose(self) -> ComposeResult:
        if self.grouped:
            yield Static(id="chips")
        yield Vertical(id="body")

    def on_mount(self) -> None:
        self.show(self.index)

    def chips(self) -> str:
        parts = []
        for i, qid in enumerate(self.qids):
            q = self.store.get(qid)
            mark = "✓" if q and q.draft_json else "●"
            name = escape(q.header if q and q.header else f"Q{qid}")
            chip = f"{mark} {name}"
            parts.append(f"[reverse b] {chip} [/]" if i == self.index else f"[$foreground]{chip}[/]")
        label = "Submit"
        parts.append(f"[reverse b] {label} [/]" if self.index == self.review_index else f"[$foreground]{label}[/]")
        return "[$text-muted]←[/]  " + "  ".join(parts) + "  [$text-muted]→[/]"

    @guarded
    def show(self, index: int) -> None:
        if not self.grouped:
            index = 0
        self.index = index % (len(self.qids) + 1)
        body = self.query_one("#body")
        body.remove_children()
        if self.index == self.review_index:
            view = ReviewView(self.store, self.qids)
        else:
            q = self.store.get(self.qids[self.index])
            view = QuestionView(q, in_group=self.grouped, last=not self.grouped or self.index == len(self.qids) - 1)
        body.mount(view)
        view.focus()
        if self.grouped:
            self.query_one("#chips", Static).update(self.chips())

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        return self.grouped and not _typing(self)

    def action_switch(self, delta: int) -> None:
        self.show(self.index + delta)

    @guarded
    def on_question_view_answered(self, event: QuestionView.Answered) -> None:
        if not self.grouped:  # a single question answers and sends at once, like Claude Code
            d = event.detail or {}
            _send(self.app, self.store, [event.qid],
                  lambda s: [x for x in [service.answer(s, event.qid, d.get("text"), choices=d.get("choices"),
                                                         notes=d.get("notes"))] if x])
            return
        if not self.store.set_draft(event.qid, event.detail):
            self.notify(f"Q{event.qid} is no longer open", severity="warning")
        if event.detail is None:
            self.query_one("#chips", Static).update(self.chips())
            return
        self.show(self.index + 1)  # Claude Code moves to the next tab (the last one: Submit)

    def on_question_view_chat(self, event: QuestionView.Chat) -> None:
        q = event.q
        err = herdr.focus_agent(q.pane_id) if q.pane_id else "no pane"
        if err:
            self.notify(f"could not focus {q.pane_id}: {err}", severity="warning")
        else:
            self.app.exit()  # the question stays open; discuss it in the agent's chat

    def on_review_view_edit(self, event: ReviewView.Edit) -> None:
        self.show(self.qids.index(event.qid))

    @guarded
    def on_review_view_submit(self, event: ReviewView.Submit) -> None:
        _send(self.app, self.store, event.qids, lambda s: service.submit(s, event.qids))


class ReviewScreen(Screen):
    """Every drafted answer, across groups and agents (``r`` on the list)."""

    CSS = "ReviewScreen { padding: 1 2; }" + VIEW_CSS

    def __init__(self, store: Store):
        super().__init__()
        self.store = store

    def compose(self) -> ComposeResult:
        yield ReviewView(self.store, None)

    def on_mount(self) -> None:
        self.query_one(ReviewView).focus()

    def on_screen_resume(self) -> None:
        view = self.query_one(ReviewView)
        view.reload()
        view.focus()

    @guarded
    def on_review_view_edit(self, event: ReviewView.Edit) -> None:
        q = self.store.get(event.qid)
        group = [g.id for g in self.store.group(q.group)]
        self.app.push_screen(GroupScreen(self.store, group, group.index(q.id)))

    @guarded
    def on_review_view_submit(self, event: ReviewView.Submit) -> None:
        _send(self.app, self.store, event.qids, lambda s: service.submit(s, event.qids))


class InboxApp(App):
    TITLE = "herdr inbox"
    CSS = """
    DataTable { height: 1fr; }
    #detail-box { height: 2fr; border-top: solid $accent; padding: 0 1; }
    """
    BINDINGS = [
        Binding("enter", "open", "Answer"),
        Binding("a", "open", "Answer", show=False),
        Binding("r", "review", "Review drafts"),
        Binding("d", "dismiss", "Dismiss"),
        Binding("g", "goto", "Go to pane"),
        Binding("h", "history", "History"),
        Binding("q", "quit", "Close"),
        Binding("escape", "quit", "Close", show=False),
    ]

    def __init__(self, store: Store | None = None):
        super().__init__()
        self.store = store or Store()
        self.history = False
        self.groups: list[list[Question]] = []

    @property
    def base(self) -> Screen:
        """The list screen (``self.query_one`` would search a pushed screen)."""
        return self.screen_stack[0]

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        # List keys (d, g, h, q, ...) must not act behind an open question or review.
        return self.screen is self.base

    def compose(self) -> ComposeResult:
        yield DataTable(id="list", cursor_type="row", zebra_stripes=True)
        with VerticalScroll(id="detail-box"):
            yield Static(id="detail")
        yield Footer()

    def on_mount(self) -> None:
        table = self.base.query_one(DataTable)
        table.add_columns("Q", "Tab", "Asked by", "Age", "Rep", "State", "Question")
        self.refresh_list()
        self.set_interval(POLL_SECONDS, self.refresh_list)

    # -- data ----------------------------------------------------------------

    @property
    def current(self) -> list[Question] | None:
        table = self.base.query_one(DataTable)
        if not self.groups or table.cursor_row is None or table.cursor_row >= len(self.groups):
            return None
        return self.groups[table.cursor_row]

    @guarded
    def refresh_list(self) -> None:
        keep = self.current[0].group if self.current else None
        questions = self.store.list(include_closed=self.history)
        if self.history:
            self.groups = [[q] for q in questions]
        else:  # one row per group, newest group first
            by_group: dict[int, list[Question]] = {}
            for q in sorted(questions, key=lambda q: q.id):
                by_group.setdefault(q.group, []).append(q)
            self.groups = sorted(by_group.values(), key=lambda g: -g[-1].id)
        table = self.base.query_one(DataTable)
        table.clear()
        for g in self.groups:
            q = g[0]
            drafted = sum(1 for x in g if x.draft_json)
            if len(g) == 1:
                state = _state(q)
            else:
                state = f"{drafted}/{len(g)} drafted" if drafted else f"{len(g)} open"
            text = service.first_line(q.question, 120) + (f" (+{len(g) - 1} more)" if len(g) > 1 else "")
            table.add_row(
                _refs(g), q.tab_label or q.pane_id or "-", q.asked_by or "", _age(q.created_at),
                str(q.repeat_count + 1) if q.repeat_count else "", state, text,
                key=f"{q.group}-{q.id}",
            )
        if keep is not None:
            for row, g in enumerate(self.groups):
                if g[0].group == keep:
                    table.move_cursor(row=row)
        drafts = len(self.store.drafted())
        self.sub_title = f"{'history' if self.history else 'open'}: {len(questions)}" + (
            f" · {drafts} drafted (r to review)" if drafts else "")
        self.show_detail()

    def show_detail(self) -> None:
        g = self.current
        detail = self.base.query_one("#detail", Static)
        if g is None:
            detail.update("No open questions." if not self.history else "Nothing here.")
            return
        parts = []
        for q in g:
            head = f"[b]{q.ref}[/b]" + (f" [reverse] {escape(q.header)} [/]" if q.header else "")
            parts += [head + f" · {escape(q.tab_label or q.pane_id or 'no pane')}"
                      + (f" · {escape(q.asked_by)}" if q.asked_by else "")
                      + f" · {_age(q.created_at)} ago · {_state(q)}",
                      escape(q.question)]
            if q.context and q is g[0]:
                parts += ["[dim]Context[/dim]", escape(q.context)]
            if q.options:
                kind = "pick any" if q.multi_select else "pick one"
                parts.append(f"[dim]Options ({kind}):[/dim] " + " · ".join(escape(o["label"]) for o in q.options))
            if q.draft_json:
                parts.append(f"[i]Draft: {escape(_draft_line(q))}[/i]")
            if q.answer:
                parts.append(f"[dim]Answer:[/dim] {escape(q.answer)}")
            if q.delivery_error:
                parts.append(f"[red]Not delivered:[/red] {escape(q.delivery_error)} (available via check_answers)")
            parts.append("")
        detail.update("\n".join(parts))

    def on_data_table_row_highlighted(self, _event: DataTable.RowHighlighted) -> None:
        self.show_detail()

    def on_data_table_row_selected(self, _event: DataTable.RowSelected) -> None:
        self.action_open()

    # -- actions -------------------------------------------------------------

    def action_open(self) -> None:
        g = self.current
        if not g or g[0].status != "open" or self.screen is not self.base:
            return
        start = next((i for i, q in enumerate(g) if not q.draft_json), len(g))
        self.push_screen(GroupScreen(self.store, [q.id for q in g], start))

    def action_review(self) -> None:
        self.push_screen(ReviewScreen(self.store))

    @guarded
    def action_dismiss(self) -> None:
        g = self.current
        closed = [q.ref for q in g or [] if service.close(self.store, q.id, "dismissed")]
        if closed:
            self.notify(f"{', '.join(closed)} dismissed")
            self.refresh_list()

    def action_goto(self) -> None:
        g = self.current
        if g and g[0].pane_id:
            err = herdr.focus_agent(g[0].pane_id)
            if err:
                self.notify(f"could not focus {g[0].pane_id}: {err}", severity="warning")
            else:
                self.exit()

    async def action_quit(self) -> None:
        # let an in-flight send finish before the popup (and its process) goes away
        await self.workers.wait_for_complete()
        self.exit()

    def action_history(self) -> None:
        self.history = not self.history
        self.refresh_list()
