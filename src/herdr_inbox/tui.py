"""Textual inbox, opened by herdr as a popup (``inbox tui``).

Screens:
- the list: open questions across tabs, one row per group (questions an agent
  filed together), with how many are drafted;
- a group view that looks and behaves like Claude Code's AskUserQuestion: a
  chip per question plus a Review chip, ←/→ between them. Picking or typing an
  answer stores a *draft* in the database (it survives closing the popup);
  nothing is sent until Submit on a Review;
- the global review (``r``): every draft across groups and agents.

A single question is simply a group of one.
"""

from __future__ import annotations

from rich.markup import escape
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Vertical, VerticalScroll
from textual.message import Message
from textual.screen import Screen
from textual.widgets import DataTable, Footer, Input, Static

from . import herdr, service
from .cli import _age
from .store import Question, Store

POLL_SECONDS = 2.0
TYPE_ROW = "Type something…"


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


def _back(app: App) -> None:
    """Leave the current view; the list reflects drafts made there right away."""
    app.pop_screen()
    if isinstance(app, InboxApp) and app.screen is app.base:
        app.refresh_list()


def _typing(screen: Screen) -> bool:
    return isinstance(screen.focused, Input)


class QuestionView(Vertical, can_focus=True):
    """One question: ❯ cursor over numbered options, notes, a free-text row."""

    class Drafted(Message):
        def __init__(self, qid: int, detail: dict | None) -> None:
            super().__init__()
            self.qid, self.detail = qid, detail

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

    def __init__(self, q: Question):
        super().__init__()
        self.q = q
        self.options = q.options or []
        labels = [o["label"] for o in self.options]
        draft = q.draft_json or {}
        self.picked = {labels.index(c) for c in draft.get("choices") or [] if c in labels}
        self.notes = {labels.index(k): v for k, v in (draft.get("notes") or {}).items() if k in labels}
        self.text = draft.get("text")
        self.cursor = len(self.options) if self.text else (min(self.picked) if self.picked else 0)
        self.editing: tuple[str, int] | None = None  # ("note", row) or ("text", row)

    @property
    def rows(self) -> int:
        return len(self.options) + 1  # + the free-text row

    def compose(self) -> ComposeResult:
        q = self.q
        meta = f"{q.ref} · {q.tab_label or q.pane_id or 'no pane'}" + (f" · {q.asked_by}" if q.asked_by else "")
        yield Static(escape(meta), classes="meta")
        yield Static(f"[b]{escape(q.question)}[/b]", classes="question")
        if q.context:
            yield Static(escape(q.context), classes="context")
        with VerticalScroll(classes="choices"):
            for i in range(self.rows):
                yield Static(id=f"row-{i}", classes="row")
        yield Static(classes="hint")

    def on_mount(self) -> None:
        self.render_rows()

    def hint(self) -> str:
        if self.editing:
            return "Enter save · Esc cancel"
        pick = "Space toggle · Enter save picks" if self.q.multi_select else "Enter select"
        return f"↑/↓ move · 1-9 jump · {pick} · n note · x clear · ←/→ question · Esc back"

    def row_text(self, i: int) -> str:
        here = i == self.cursor
        arrow = "[b $accent]❯[/]" if here else " "
        if i == len(self.options):
            label = f"[b $accent]{TYPE_ROW}[/]" if here else f"[dim]{TYPE_ROW}[/]"
            line = f"{arrow} {i + 1}. {label}"
            return line + (f"\n      [i]draft: {escape(self.text)}[/]" if self.text else "")
        opt = self.options[i]
        if self.q.multi_select:
            box = "[x] " if i in self.picked else "[ ] "
        else:
            box = "✓ " if i in self.picked else ""
        label = escape(opt["label"])
        lines = [f"{arrow} {i + 1}. {escape(box)}" + (f"[b $accent]{label}[/]" if here else label)]
        if opt.get("description"):
            lines.append(f"      [dim]{escape(opt['description'])}[/]")
        if self.notes.get(i):
            lines.append(f"      [i]note: {escape(self.notes[i])}[/]")
        return "\n".join(lines)

    def render_rows(self) -> None:
        for i in range(self.rows):
            self.query_one(f"#row-{i}", Static).update(self.row_text(i))
        self.query_one(".hint", Static).update(self.hint())

    # -- actions -------------------------------------------------------------

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        # While a note or the free-text box is open, keys belong to it (Esc still cancels).
        return action == "back" or not _typing(self.screen)

    def action_move(self, delta: int) -> None:
        self.cursor = (self.cursor + delta) % self.rows
        self.render_rows()

    def action_jump(self, n: int) -> None:
        if 1 <= n <= self.rows:
            self.cursor = n - 1
            self.render_rows()

    def action_toggle(self) -> None:
        if self.q.multi_select and self.cursor < len(self.options):
            self.picked ^= {self.cursor}
            self.render_rows()

    def action_select(self) -> None:
        if self.cursor == len(self.options):
            self._open_input("text", self.cursor, self.text or "")
        elif self.q.multi_select:
            if not self.picked:
                self.notify("Space picks options; Enter saves them", severity="warning")
                return
            self._draft()
        else:
            self.picked, self.text = {self.cursor}, None
            self._draft()

    def action_note(self) -> None:
        if self.cursor < len(self.options):
            self._open_input("note", self.cursor, self.notes.get(self.cursor, ""))

    def action_clear(self) -> None:
        self.picked, self.notes, self.text = set(), {}, None
        self.render_rows()
        self.post_message(self.Drafted(self.q.id, None))

    def action_back(self) -> None:
        if self.editing:
            self._close_input()
        else:
            _back(self.app)

    def _draft(self) -> None:
        labels = [self.options[i]["label"] for i in sorted(self.picked)]
        notes = {self.options[i]["label"]: n for i, n in self.notes.items()}
        detail = service.make_detail(None, labels, notes) if labels else service.make_detail(self.text)
        self.render_rows()
        self.post_message(self.Drafted(self.q.id, detail))

    # -- inline inputs -------------------------------------------------------

    def _open_input(self, kind: str, row: int, value: str = "") -> None:
        self._close_input()
        self.editing = (kind, row)
        placeholder = "Note for this option" if kind == "note" else f"Answer to {self.q.ref}"
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
        kind, row = self.editing or ("", 0)
        value = event.value.strip()
        self._close_input()
        if kind == "note":
            if value:
                self.notes[row] = value
            else:
                self.notes.pop(row, None)
            self.render_rows()
            if row in self.picked and self.q.draft_json is not None:
                self._draft()  # keep a saved pick's note in its draft
        elif kind == "text" and value:
            self.picked, self.text = set(), value
            self._draft()


class ReviewView(Vertical, can_focus=True):
    """Drafted answers with their questions; Enter edits one, x drops it, Submit sends them."""

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
        Binding("x", "drop", show=False),
        Binding("escape", "back", show=False),
    ]

    def __init__(self, store: Store, qids: list[int] | None = None):
        """``qids``: one group's questions; None means every draft (the global review)."""
        super().__init__()
        self.store = store
        self.qids = qids
        self.items: list[Question] = []
        self.cursor: int | None = None  # starts on Submit

    def compose(self) -> ComposeResult:
        yield Static(classes="question")
        yield VerticalScroll(Static(id="review-body"), classes="choices")
        yield Static("↑/↓ move · Enter edit / submit · x drop draft · ←/→ question · Esc back", classes="hint")

    def on_mount(self) -> None:
        self.reload()

    def reload(self) -> None:
        if self.qids is None:
            self.items = self.store.drafted()
        else:
            self.items = [q for q in (self.store.get(i) for i in self.qids) if q and q.status == "open"]
        self.cursor = len(self.items) if self.cursor is None else min(self.cursor, len(self.items))
        self.render_rows()

    @property
    def drafted(self) -> list[Question]:
        return [q for q in self.items if q.draft_json]

    def render_rows(self) -> None:
        title = "Review your answers" if self.qids is not None else "Review all drafted answers"
        self.query_one(".question", Static).update(f"[b]{title}[/b]")
        lines = []
        for i, q in enumerate(self.items):
            arrow = "[b $accent]❯[/]" if i == self.cursor else " "
            where = q.tab_label or q.pane_id or "-"
            lines.append(f"{arrow} {q.ref} · {escape(where)} · {escape(service.first_line(q.question, 70))}")
            if q.draft_json:
                lines.append(f"      → [b]{escape(_draft_line(q))}[/b]")
            else:
                lines.append("      [dim]no answer yet: stays open[/dim]")
        n = len(self.drafted)
        here = self.cursor == len(self.items)
        label = f"Submit {n} answer{'s' if n != 1 else ''}" if n else "Nothing drafted yet"
        arrow = "[b $accent]❯[/]" if here else " "
        lines.append(f"{arrow} " + (f"[b $accent]{label}[/]" if here else label))
        self.query_one("#review-body", Static).update("\n".join(lines))

    def action_move(self, delta: int) -> None:
        self.cursor = (self.cursor + delta) % (len(self.items) + 1)
        self.render_rows()

    def action_select(self) -> None:
        if self.cursor < len(self.items):
            self.post_message(self.Edit(self.items[self.cursor].id))
        elif self.drafted:
            self.post_message(self.Submit([q.id for q in self.drafted]))

    def action_drop(self) -> None:
        if self.cursor < len(self.items):
            self.store.set_draft(self.items[self.cursor].id, None)
            self.reload()

    def action_back(self) -> None:
        _back(self.app)


VIEW_CSS = """
.meta { color: $text-muted; }
.question { margin: 1 0; }
.context { color: $text-muted; margin-bottom: 1; }
.choices { height: 1fr; }
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
            app.notify(f"{wanted - len(done)} drafted question(s) had closed meanwhile", severity="warning")
    app.refresh_list()


class GroupScreen(Screen):
    """The questions of one group as chips (✓ drafted · ● not yet) plus a Review chip."""

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
    def review_index(self) -> int:
        return len(self.qids)

    def compose(self) -> ComposeResult:
        yield Static(id="chips")
        yield Vertical(id="body")

    def on_mount(self) -> None:
        self.show(self.index)

    def chips(self) -> str:
        parts = []
        for i, qid in enumerate(self.qids):
            q = self.store.get(qid)
            mark = "✓" if q and q.draft_json else ("·" if q and q.status != "open" else "●")
            name = escape((q.header if q and q.header else f"Q{qid}"))
            chip = f" {mark} {name} "
            parts.append(f"[reverse b]{chip}[/]" if i == self.index else chip)
        review = " Review "
        parts.append(f"[reverse b]{review}[/]" if self.index == self.review_index else review)
        return "←  " + " ".join(parts) + "  →"

    def show(self, index: int) -> None:
        self.index = index % (len(self.qids) + 1)
        body = self.query_one("#body")
        body.remove_children()
        if self.index == self.review_index:
            view = ReviewView(self.store, self.qids)
        else:
            q = self.store.get(self.qids[self.index])
            view = QuestionView(q)
        body.mount(view)
        view.focus()
        self.query_one("#chips", Static).update(self.chips())

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        return not _typing(self)

    def action_switch(self, delta: int) -> None:
        self.show(self.index + delta)

    def on_question_view_drafted(self, event: QuestionView.Drafted) -> None:
        if not self.store.set_draft(event.qid, event.detail):
            self.notify(f"Q{event.qid} is no longer open", severity="warning")
        if event.detail is None:
            self.query_one("#chips", Static).update(self.chips())
            return
        # next question without a draft, else Review
        for offset in range(1, len(self.qids) + 1):
            i = (self.index + offset) % (len(self.qids) + 1)
            if i == self.review_index:
                break
            q = self.store.get(self.qids[i])
            if q and q.status == "open" and not q.draft_json:
                self.show(i)
                return
        self.show(self.review_index)

    def on_review_view_edit(self, event: ReviewView.Edit) -> None:
        self.show(self.qids.index(event.qid))

    def on_review_view_submit(self, event: ReviewView.Submit) -> None:
        done = service.submit(self.store, event.qids)
        app = self.app
        app.pop_screen()
        _after_submit(app, done, len(event.qids))


class ReviewScreen(Screen):
    """Every drafted answer, across groups and agents."""

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

    def on_review_view_edit(self, event: ReviewView.Edit) -> None:
        q = self.store.get(event.qid)
        group = [g.id for g in self.store.group(q.group)]
        self.app.push_screen(GroupScreen(self.store, group, group.index(q.id)))

    def on_review_view_submit(self, event: ReviewView.Submit) -> None:
        done = service.submit(self.store, event.qids)
        app = self.app
        app.pop_screen()
        _after_submit(app, done, len(event.qids))


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

    def action_history(self) -> None:
        self.history = not self.history
        self.refresh_list()
