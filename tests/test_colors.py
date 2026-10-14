"""Claude Code's colour roles: suggestion (focus), success (chosen answers), warning (only warnings)."""

from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.tui import CC_COLORS, InboxApp, QuestionView, ReviewView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")


async def test_roles_follow_claude_code_in_dark_and_light(store, fake_herdr):
    service.ask_group(store, A, [{"question": "One?", "options": ["a", "b"]}, {"question": "Two?", "options": ["c"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        assert {k: app.get_css_variables()[k] for k in CC_COLORS["dark"]} == CC_COLORS["dark"]
        v = app.query_one(QuestionView)
        assert v.row_text(0).startswith("[$inbox-suggestion]❯[/]")  # focus: suggestion
        await pilot.press("2", "enter")  # One = b -> Two
        await pilot.press("left")
        assert "[$inbox-success]✔[/]" in app.query_one(QuestionView).row_text(1)  # chosen: success
        await pilot.press("tab", "tab")
        body = app.query_one(ReviewView)
        lines = [ln for ln in body.query_one("#review-body").render().plain.split("\n")]
        assert any(ln.strip() == "→ b" for ln in lines)
        app.theme = "textual-light"
        await pilot.pause()
        assert {k: app.get_css_variables()[k] for k in CC_COLORS["light"]} == CC_COLORS["light"]


def test_answers_are_success_and_warning_is_reserved():
    import inspect

    from herdr_inbox import tui

    src = inspect.getsource(tui)
    assert '[$inbox-success]→ {escape(_draft_line(q))}' in src  # Review: chosen answers in green
    for warning in ("You have not answered all questions", "not delivered", "was closed"):
        assert warning in src
    # the old accent (orange in the default theme) is gone from the question UI
    assert "$accent" not in src


async def test_tick_glyph_and_boxes_match_claude_code(store, fake_herdr):
    """figures.tick (✔) everywhere; multi-select boxes are "[✔]" in success, "[ ]" otherwise;
    a tab's mark sits outside the highlighted label (no coloured block)."""
    from herdr_inbox.tui import TICK

    assert TICK == "✔"
    service.ask_group(store, A, [{"question": "Pick?", "multi_select": True, "options": ["a", "b"]},
                                 {"question": "Two?", "options": ["c"]}])
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("space")
        v = app.query_one(QuestionView)
        assert "[$inbox-success]\\[✔][/]" in v.row_text(0) and "\\[ ]" in v.row_text(1)
        await pilot.press("down", "down", "down", "enter")  # Next: drafted, on to Two
        await pilot.press("left")
        chip = app.query_one("#chips > .current")
        rendered = str(chip.render())
        assert rendered.startswith("✔") and "Q1" in rendered


async def test_list_title_row(store, fake_herdr):
    service.ask(store, A, "one?")
    service.ask(store, Caller("w1:p2", "w1:t2", "tab-B", "sess-A", "/r"), "two?")
    app = InboxApp(store)
    async with app.run_test() as pilot:
        await pilot.pause()
        assert str(app.query_one("#list-title").render()) == "Open questions · 2 groups"
