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
  asking agent's pane and leaves the question open; ``n`` adds notes; ``d``
  dismisses the current question's group; Esc closes the popup without
  sending;
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
from textual.screen import Screen
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
    if not app.order and done and not failed:
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


class InboxApp(App):
    """Every open question as a tab, plus Submit when there are several."""

    TITLE = "herdr inbox"
    CSS = """
    Screen { padding: 1 2; }
    #chips { height: 1; margin-bottom: 1; scrollbar-size-horizontal: 0; }
    #chips > Static { width: auto; margin-right: 2; }
    #chips > .sep { color: $text-muted; }
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
        Binding("escape", "quit", show=False),
    ]

    def __init__(self, store: Store | None = None, focus_pane: str | None = None):
        super().__init__()
        self.store = store or Store()
        self.focus_pane = focus_pane
        self.order: list[int] = []
        self.index = 0

    # -- state -------------------------------------------------------------------

    @property
    def multi(self) -> bool:
        return len(self.order) > 1

    @property
    def review_index(self) -> int:
        return len(self.order)

    @property
    def current_qid(self) -> int | None:
        return self.order[self.index] if self.index < len(self.order) else None

    def compose(self) -> ComposeResult:
        yield HorizontalScroll(id="chips")
        yield Vertical(id="body")
        yield Static("No open questions", id="empty")

    def on_mount(self) -> None:
        if self.focus_pane is None:
            self.focus_pane = os.environ.get("INBOX_FOCUS_PANE") or herdr.focused_pane()
        self.order = tab_order(self._open(), self.focus_pane)
        self.show(0)
        self.set_interval(POLL_SECONDS, self.poll)

    def _open(self) -> list[Question]:
        try:
            return self.store.list()
        except sqlite3.OperationalError:
            self.notify(BUSY, severity="warning")
            return []

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        if action in ("switch", "dismiss"):
            return bool(self.order) and not isinstance(self.focused, Input) and (action != "switch" or self.multi)
        return True

    # -- rendering -----------------------------------------------------------------

    def show(self, index: int) -> None:
        self.set_class(not self.order, "empty")
        body = self.query_one("#body")
        body.remove_children()
        if not self.order:
            self.query_one("#chips").remove_children()
            return
        self.index = index % (len(self.order) + 1) if self.multi else 0
        qid = self.current_qid
        if qid is None:
            view = ReviewView(self.store, list(self.order))
        else:
            view = QuestionView(self.store.get(qid), in_group=self.multi, last=self.index == len(self.order) - 1)
        body.mount(view)
        view.focus()
        self.render_chips()

    def render_chips(self) -> None:
        row = self.query_one("#chips", HorizontalScroll)
        row.remove_children()
        if not self.multi:
            return
        chips, current, last_tab = [], None, object()
        for i, qid in enumerate(self.order):
            q = self.store.get(qid)
            if q is None:
                continue
            tab = q.tab_label or q.pane_id
            if tab != last_tab:  # agent boundary
                chips.append(Static(f"│ {escape(tab or 'no pane')}", classes="sep"))
                last_tab = tab
            mark = "✓" if q.draft_json else "●"
            text = f"{mark} {escape(q.header or q.ref)}"
            here = i == self.index
            chip = Static(f"[reverse b] {text} [/]" if here else f"[$foreground]{text}[/]",
                          classes="current" if here else "")
            chips.append(chip)
            if i == self.index:
                current = chip
        on_submit = self.current_qid is None
        submit = Static("[reverse b] Submit [/]" if on_submit else "[$foreground]Submit[/]",
                        classes="current" if on_submit else "")
        chips.append(submit)
        row.mount_all(chips)
        target = current or submit
        self.call_after_refresh(target.scroll_visible, animate=False)

    @guarded
    def poll(self) -> None:
        """Pick up new questions (appended, the current tab stays) and drop closed ones."""
        open_qs = self.store.list()
        open_ids = {q.id for q in open_qs}
        keep = [qid for qid in self.order if qid in open_ids]
        new = tab_order([q for q in open_qs if q.id not in set(self.order)], None)
        if keep == self.order and not new:
            if self.multi:
                self.render_chips()  # drafts may have changed (✓ / ●)
            return
        current = self.current_qid
        was_review = self.multi and current is None
        was_multi = self.multi
        self.order = keep + new
        if current in self.order and was_multi == self.multi:
            self.index = self.order.index(current)
            self.render_chips()
        elif was_review and self.multi:
            self.index = self.review_index
            self.query_one(ReviewView).reload()
            self.render_chips()
        else:
            if current is not None and current not in self.order:
                self.notify(f"Q{current} was closed meanwhile", severity="warning")
            self.show(min(self.index, len(self.order)))

    # -- actions -------------------------------------------------------------------

    def action_switch(self, delta: int) -> None:
        self.show(self.index + delta)

    def on_key(self, event) -> None:
        if not self.order:  # empty state: any key closes
            event.stop()
            self.exit()

    @guarded
    def on_question_view_answered(self, event: QuestionView.Answered) -> None:
        if not self.multi:  # the only open question: answer and send at once, like Claude Code
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
        self.show(self.index + 1)  # Claude Code moves to the next tab (the last one: Submit)

    def on_question_view_chat(self, event: QuestionView.Chat) -> None:
        q = event.q
        err = herdr.focus_agent(q.pane_id) if q.pane_id else "no pane"
        if err:
            self.notify(f"could not focus {q.pane_id}: {err}", severity="warning")
        else:
            self.exit()  # the question stays open; discuss it in the agent's chat

    def on_review_view_edit(self, event: ReviewView.Edit) -> None:
        self.show(self.order.index(event.qid))

    def on_review_view_submit(self, event: ReviewView.Submit) -> None:
        _send(self, self.store, event.qids, lambda s: service.submit(s, event.qids))

    @guarded
    def action_dismiss(self) -> None:
        qid = self.current_qid
        q = self.store.get(qid) if qid else None
        if q is None:
            return
        closed = [g.ref for g in self.store.group(q.group) if service.close(self.store, g.id, "dismissed")]
        if closed:
            self.notify(f"{', '.join(closed)} dismissed")
        self.poll()

    async def action_quit(self) -> None:
        # let an in-flight send finish before the popup (and its process) goes away
        await self.workers.wait_for_complete()
        self.exit()
