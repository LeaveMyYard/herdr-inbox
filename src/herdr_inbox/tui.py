"""Textual inbox, opened by herdr as a popup (``inbox tui``)."""

from __future__ import annotations

from rich.markup import escape
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import VerticalScroll
from textual.widgets import DataTable, Footer, Input, Static

from . import herdr, service
from .cli import _age
from .store import Question, Store

POLL_SECONDS = 2.0


def _state(q: Question) -> str:
    if q.status != "answered":
        return q.status
    if q.delivered_at:
        return "answered ✓"
    return "answered !" if q.delivery_error else "answered"


class InboxApp(App):
    TITLE = "herdr inbox"
    CSS = """
    DataTable { height: 1fr; }
    #detail-box { height: 2fr; border-top: solid $accent; padding: 0 1; }
    #answer { dock: bottom; display: none; }
    #answer.visible { display: block; }
    """
    BINDINGS = [
        Binding("a", "answer", "Answer"),
        Binding("enter", "answer", "Answer", show=False),
        Binding("d", "dismiss", "Dismiss"),
        Binding("g", "goto", "Go to pane"),
        Binding("h", "history", "History"),
        Binding("q", "quit", "Close"),
        Binding("escape", "escape", "Close", show=False),
        *[Binding(str(n), f"pick({n})", show=False) for n in range(1, 10)],
    ]

    def __init__(self, store: Store | None = None):
        super().__init__()
        self.store = store or Store()
        self.history = False
        self.questions: list[Question] = []

    def compose(self) -> ComposeResult:
        yield DataTable(id="list", cursor_type="row", zebra_stripes=True)
        with VerticalScroll(id="detail-box"):
            yield Static(id="detail")
        yield Input(id="answer", placeholder="Answer, Enter sends, Esc cancels")
        yield Footer()

    def on_mount(self) -> None:
        table = self.query_one(DataTable)
        table.add_columns("Q", "Tab", "Asked by", "Age", "Rep", "State", "Question")
        self.refresh_list()
        self.set_interval(POLL_SECONDS, self.refresh_list)

    # -- data ----------------------------------------------------------------

    @property
    def current(self) -> Question | None:
        table = self.query_one(DataTable)
        if not self.questions or table.cursor_row is None or table.cursor_row >= len(self.questions):
            return None
        return self.questions[table.cursor_row]

    def refresh_list(self) -> None:
        keep = self.current.id if self.current else None
        self.questions = self.store.list(include_closed=self.history)
        table = self.query_one(DataTable)
        table.clear()
        for q in self.questions:
            table.add_row(
                q.ref, q.tab_label or q.pane_id or "-", q.asked_by or "", _age(q.created_at),
                str(q.repeat_count + 1) if q.repeat_count else "", _state(q),
                service.first_line(q.question, 120), key=str(q.id),
            )
        if keep is not None:
            for row, q in enumerate(self.questions):
                if q.id == keep:
                    table.move_cursor(row=row)
        self.sub_title = f"{'history' if self.history else 'open'}: {len(self.questions)}"
        self.show_detail()

    def show_detail(self) -> None:
        q = self.current
        detail = self.query_one("#detail", Static)
        if q is None:
            detail.update("No open questions." if not self.history else "Nothing here.")
            return
        parts = [f"[b]{q.ref}[/b] · {escape(q.tab_label or q.pane_id or 'no pane')}"
                 + (f" · {escape(q.asked_by)}" if q.asked_by else "")
                 + f" · {_age(q.created_at)} ago · {_state(q)}",
                 "", escape(q.question)]
        if q.context:
            parts += ["", "[dim]Context[/dim]", escape(q.context)]
        if q.options:
            parts += ["", "[dim]Options (number key picks)[/dim]"]
            parts += [f"  [b]{i}[/b]  {escape(o)}" for i, o in enumerate(q.options, 1)]
        if q.answer:
            parts += ["", "[dim]Answer[/dim]", escape(q.answer)]
        if q.delivery_error:
            parts += ["", f"[red]Not delivered:[/red] {escape(q.delivery_error)} (available via check_answers)"]
        detail.update("\n".join(parts))

    def on_data_table_row_highlighted(self, _event: DataTable.RowHighlighted) -> None:
        self.show_detail()

    def on_data_table_row_selected(self, _event: DataTable.RowSelected) -> None:
        self.action_answer()

    # -- actions -------------------------------------------------------------

    def _open_input(self, value: str = "") -> None:
        q = self.current
        if q is None or q.status != "open":
            return
        box = self.query_one("#answer", Input)
        box.value = value
        box.add_class("visible")
        box.focus()

    def _close_input(self) -> None:
        box = self.query_one("#answer", Input)
        box.remove_class("visible")
        box.value = ""
        self.query_one(DataTable).focus()

    def action_answer(self) -> None:
        self._open_input()

    def action_pick(self, n: int) -> None:
        q = self.current
        if q and q.options and 1 <= n <= len(q.options):
            self._open_input(q.options[n - 1])

    def on_input_submitted(self, event: Input.Submitted) -> None:
        q, text = self.current, event.value.strip()
        self._close_input()
        if q is None or not text:
            return
        done = service.answer(self.store, q.id, text)
        if done is None:
            self.notify(f"{q.ref} is no longer open", severity="warning")
        elif done.delivered_at:
            self.notify(f"{q.ref} answered and delivered to {done.pane_id}")
        else:
            self.notify(f"{q.ref} answered, not delivered: {done.delivery_error}", severity="warning")
        self.refresh_list()

    def action_dismiss(self) -> None:
        q = self.current
        if q and service.close(self.store, q.id, "dismissed"):
            self.notify(f"{q.ref} dismissed")
            self.refresh_list()

    def action_goto(self) -> None:
        q = self.current
        if q and q.pane_id:
            err = herdr.focus_agent(q.pane_id)
            if err:
                self.notify(f"could not focus {q.pane_id}: {err}", severity="warning")
            else:
                self.exit()

    def action_history(self) -> None:
        self.history = not self.history
        self.refresh_list()

    def action_escape(self) -> None:
        if self.query_one("#answer", Input).has_class("visible"):
            self._close_input()
        else:
            self.exit()

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        # While typing an answer, letters belong to the input, not to the bindings.
        boxes = self.query("#answer")
        if action != "escape" and boxes and boxes.first().has_focus:
            return False
        return True
