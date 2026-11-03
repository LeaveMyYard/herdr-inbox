"""The scenarios of the popup transition golden (tools/capture_popup_transitions.py).

Each one is a Python popup test replayed as data: the same database, the same keys, with
the REAL delays on a held clock. The name says which test it comes from
(``<file without test_>.<test without test_>``); ``edge.*`` scenarios pin down behaviour no
test asserts but the model must still match (keys inside an open note box, a confirmation
under Tab, ...).

Keys are 600 ms apart (``keys``), past every window, unless a scenario presses them
``fast`` (50 ms apart): those are the keys that must be ignored.
"""

from __future__ import annotations

import contextlib
import os


@contextlib.contextmanager
def env(**values):
    """FAKE_HERDR_* switches for the fake herdr while a scenario's database is set up."""
    old = {k: os.environ.get(k) for k in values}
    os.environ.update(values)
    try:
        yield
    finally:
        for k, v in old.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def SCENARIOS(h):  # noqa: N802, C901 - the list the capture runs
    from herdr_inbox import alerts, service
    from herdr_inbox.service import Caller

    A, B, C = h.callers()
    key, keys, typed, poll, sent, focused, wait = h.key, h.keys, h.typed, h.poll, h.sent, h.focused, h.wait
    Run = h.Run
    DB = [{"label": "Postgres", "description": "keeps the current ORM"}, {"label": "SQLite", "description": "single file"},
          {"label": "DuckDB"}]
    out = []

    def add(name, doc, setup, steps, focus="w1:p1", deeplink=None):
        out.append((Run(name, doc, setup, focus, deeplink), list(steps)))

    def fast(*names):
        return keys(*names, gap=50)

    def seq(*parts):
        steps = []
        for p in parts:
            steps += p if isinstance(p, list) else [p]
        return steps

    def failed(store, caller, text, answer, **kw):
        qid = service.ask(store, caller, text, **kw)["id"]
        with env(FAKE_HERDR_FAIL="agent prompt"):
            service.answer(store, qid, answer)
        return qid

    def gone(store, caller, text):
        qid = service.ask(store, caller, text, options=["yes"])["id"]
        with env(FAKE_HERDR_AGENT="0"):
            service.answer(store, qid, "yes")
        store.mark_agent_gone(qid, f"no live agent in pane {caller.pane_id}")
        return qid

    def blocked_alert(store):
        store.db.execute("INSERT INTO alerts (created_at, kind, severity, title, pane_id, tab_label, status)"
                         " VALUES (?, 'blocked', 'warning', 'tab-A is waiting for you', 'w1:p1', 'tab-A', 'open')",
                         (alerts.now(),))

    def close(qid, status="withdrawn"):
        return lambda s: service.close(s, qid, status)

    def ask(caller, text, **kw):
        return lambda s: service.ask(s, caller, text, **kw)

    def setup(*fns):
        def run(store):
            for fn in fns:
                fn(store)
        return run

    def group(caller, *qs, **kw):
        return lambda s: service.ask_group(s, caller, list(qs), **kw)

    def q(text, *options, **kw):
        return {"question": text, **({"options": list(options)} if options else {}), **kw}

    # -- tests/test_tui.py ------------------------------------------------------------------
    add("tui.group_list_rows_and_order", "the focused pane's group first; Enter opens it, with its own tabs",
        setup(ask(B, "B one?", header="Deploy"), group(A, q("A one?", header="DB"), q("A two?"), asked_by="planner")),
        keys("enter", "right", "right", "right"))
    add("tui.single_question_layout_and_keys", "a lone question opens directly; arrows, j/k and digits move",
        ask(A, "Which DB?", options=DB, header="DB choice"), keys("down", "j", "k", "4", "1", "down", "down", "down",
                                                                   "down", "up"))
    add("tui.single_pick_sends_immediately_and_closes", "a note, then a pick: sent at once; the popup quits",
        ask(A, "Which DB?", options=DB), seq(keys("2", "n"), typed("no server"), keys("enter", "enter")))
    add("tui.type_something_and_multi_select", "multi-select: Space and Enter toggle, typed text joins the picks",
        ask(A, "Which DBs?", options=DB, multi_select=True),
        seq(keys("enter", "3", "space", "4", "enter"), typed("also redis"), keys("enter", "down", "enter")))
    add("tui.multi_select_submit_needs_a_pick", "Submit with nothing picked only warns; Space on Other opens it",
        ask(A, "Which DBs?", options=DB, multi_select=True),
        seq(keys("5", "enter", "4", "space", "escape", "1", "space", "space", "x", "5", "enter")))
    add("tui.chat_about_this_and_escape", "Esc closes the note box only; Chat about this goes to the agent",
        ask(A, "Which DB?", options=DB),
        seq(keys("n"), typed("dq"), keys("escape", "5", "enter"), focused("w1:p1")))
    add("tui.chat_about_this_and_escape.esc", "Esc on a lone question closes the popup without sending",
        ask(A, "Which DB?", options=DB), keys("escape"))
    add("tui.empty_state_closes_on_any_key", "nothing open: any key closes", lambda s: None, keys("x"))
    add("tui.answering_drafts_and_moves_to_the_next_tab", "an answer in a group is a draft; on to the next tab",
        setup(group(A, q("One?", "a", "b", header="First"), q("Two?", "c", "d", header="Second")),
              ask(B, "Three?", options=["e"])),
        keys("enter", "right", "tab", "tab", "1", "enter", "2", "enter"))
    add("tui.submit_is_per_group_and_keeps_blanks", "Submit sends this group's drafts only; blanks stay open",
        setup(group(A, q("One?", "a", "b"), q("Two?", "c")), group(B, q("Three?"), q("Four?", "x"))),
        seq(keys("2", "enter", "enter"), typed("later"), keys("enter", "1", "enter", "enter"), sent(),
            keys("1", "enter", "1", "enter", "tab", "enter"), sent()))
    add("tui.cancel_and_drafts_survive_a_restart", "the only group opens directly; Cancel closes the popup",
        group(A, q("One?", "a", "b"), q("Two?", "c")), keys("2", "enter", "right", "2"))

    def drafted(store):
        service.ask_group(store, A, [q("One?", "a", "b"), q("Two?", "c")])
        store.set_draft(1, service.make_detail(None, ["b"], None))

    add("tui.cancel_and_drafts_survive_a_restart.again", "a drafted group opens on its first blank tab; x clears",
        drafted, keys("left", "1", "enter", "left", "x"))
    add("tui.new_groups_never_join_an_open_group", "arrivals go to the list; a closed tab drops out of the group",
        group(A, q("One?"), q("Two?")),
        seq(keys("right"), poll(ask(B, "Three?")), poll(close(1)), keys("escape")))
    add("tui.one_becomes_several_while_open", "the send mode is fixed when the group opens",
        setup(ask(A, "One?", options=["a"]), ask(A, "Spare?", options=["s"])),
        seq(keys("enter"), poll(ask(B, "Two?", options=["b"])), keys("1", "enter"), sent()))
    add("tui.dismiss_question_in_group_and_whole_group_in_list", "d asks; only y dismisses",
        setup(group(A, q("One?"), q("Two?")), group(B, q("Three?"), q("Four?"))),
        keys("enter", "d", "x", "d", "y", "escape", "down", "d", "d", "d", "y"))

    # -- tests/test_background_keys.py ----------------------------------------------------
    add("background_keys.arrival_into_the_empty_popup_does_not_take_over", "an arrival never takes over the empty view",
        lambda s: None, seq(poll(ask(A, "delete bucket?", options=["yes", "no"])), keys("enter")))
    add("background_keys.arrival_then_deliberate_tab_enables_immediate_send", "Tab to the list, Enter opens the lone question",
        lambda s: None, seq(poll(ask(A, "delete bucket?", options=["yes", "no"])), keys("tab", "enter", "2", "enter")))
    add("background_keys.typing_after_the_question_closes_never_reaches_app_keys",
        "typing into Other while the question closes: the rest lands nowhere",
        group(B, q("branch name?", "main", "dev"), q("delete bucket?", "yes", "no")),
        seq(keys("3", "enter"), typed("use the "), poll(close(1)), typed("good one"), fast("enter", "d", "y")),
        focus="w1:p2")
    add("background_keys.keys_are_swallowed_briefly_after_a_background_view_change", "0.5 s of dropped keys",
        group(B, q("first?", "a"), q("second?", "b")),
        seq(poll(close(1)), fast("right"), key("right", gap=300), key("right", gap=200)), focus="w1:p2")
    add("background_keys.gone_tab_disables_g_and_d", "a closed tab's placeholder: g, d, Enter and digits do nothing",
        group(B, q("first?", "a"), q("second?", "b")),
        seq(poll(close(1)), keys("g", "d", "y", "enter", "1")), focus="w1:p2")
    add("background_keys.dismiss_needs_y", "a quick double d is not a confirmation",
        group(A, q("q1?", "a"), q("q2?", "b")), seq(fast("d", "d"), keys("d", "y")))

    # -- tests/test_cursor_targets.py -------------------------------------------------------
    add("cursor_targets.cancel_stays_cancel_when_a_question_arrives", "the Submit tab's cursor is a target",
        group(A, q("q1?", "a", "b"), q("q2?", "x", "y")),
        seq(keys("enter", "tab", "down"), poll(ask(A, "q3 new?", options=["m"])), keys("enter")))
    add("cursor_targets.item_cursor_follows_its_question", "a row above the cursor goes: the cursor stays on Q3",
        group(A, q("q0?"), q("q1?", "a"), q("q2?", "x")),
        seq(keys("right", "enter", "tab", "up"), poll(close(1)), keys("enter")))
    add("cursor_targets.vanished_target_parks_on_nothing", "the cursor's question goes: inert until an arrow",
        group(A, q("q1?", "a"), q("q2?", "x"), q("q3?", "y")),
        seq(keys("enter", "tab", "tab", "up"), poll(close(3)), keys("enter", "down", "up", "up")))

    # -- tests/test_group_list.py -----------------------------------------------------------
    add("group_list.opens_straight_into_the_only_group", "one group: opened directly",
        group(A, q("One?"), q("Two?")), [])
    add("group_list.opens_straight_into_the_only_group.not_with_a_warning_entry", "a ⚠ entry: the list",
        setup(group(A, q("One?"), q("Two?")), lambda s: failed(s, B, "Old?", "x")), [])
    add("group_list.closed_row_is_inert_until_the_operator_moves", "a closed row stays in place, inert",
        setup(ask(A, "first?", options=["a"]), ask(B, "second?", options=["b"]), ask(C, "third?", options=["c"])),
        seq(keys("down"), poll(close(2)), keys("enter", "g", "d", "y", "3", "enter")))
    add("group_list.cursor_target_vanishing_from_a_return_parks_inert", "a sent group's row is inert",
        setup(ask(A, "first?", options=["a"]), ask(B, "second?", options=["b"])),
        seq(keys("enter", "escape", "enter", "1", "enter"), sent(), keys("enter", "down")))
    add("group_list.g_from_the_list_goes_to_the_groups_agent", "g on a row: its agent's pane",
        setup(ask(A, "first?"), ask(B, "second?")), seq(keys("down", "g"), focused("w1:p2")))
    add("group_list.last_group_sent_closes_the_popup", "nothing left after a send: the popup quits",
        setup(ask(A, "only?", options=["a"]), ask(B, "other?", options=["b"])),
        seq(keys("enter", "1", "enter"), sent(), keys("down", "enter", "1", "enter")))

    # -- tests/test_arrivals.py -------------------------------------------------------------
    add("arrivals.tab_enter_after_arrival_never_answers_the_new_question", "nothing joins an open group",
        group(A, q("q1?", "a", "b"), q("q2?", "x", "y")),
        seq(keys("enter", "enter", "shift+tab"), poll(ask(B, "drop prod table?", options=["yes", "no"])),
            keys("tab", "enter"), sent(), keys("enter")))
    add("arrivals.arrivals_are_list_rows_and_never_move_the_cursor", "arrivals are appended; the cursor stays",
        setup(ask(A, "q1?", options=["a"]), ask(A, "q2?", options=["b"])),
        seq(keys("down"), poll(setup(ask(B, "new one?", options=["c"]), ask(B, "newer?", options=["d"]))),
            keys("enter")))
    add("arrivals.arrival_into_the_empty_view_still_opens_with_tab", "the empty view: Tab shows what arrived",
        lambda s: None, seq(poll(ask(A, "late?", options=["a"])), keys("tab", "enter")))
    add("arrivals.question_view_is_armed_only_after_a_moment", "0.4 s after a view appears, keys count",
        group(A, q("q1?", "a", "b"), q("q2?", "x", "y")),
        seq(key("2", gap=100), key("enter", gap=100), key("2", gap=450), fast("enter", "enter")))
    add("arrivals.list_and_lone_question_are_armed_too", "Enter right behind the open never sends",
        setup(ask(A, "lone?", options=["yes", "no"]), ask(B, "other?", options=["x"])),
        seq(key("enter", gap=100), key("enter", gap=450), fast("enter"), key("enter", gap=450)))

    # -- tests/test_race.py -----------------------------------------------------------------
    add("race.withdraw_mid_typing_never_answers_the_other_agent", "a withdrawn question's view stays inert",
        group(B, q("Q-for-B: which branch name?", "main", "dev"), q("Q-also: delete the staging bucket data?", "yes", "no")),
        seq(keys("3", "enter"), typed("use feat"), poll(close(1)), typed("ure-x"), fast("enter", "1", "enter"),
            keys("right")), focus="w1:p2")
    add("race.cursor_on_an_option_when_the_question_closes", "Enter on a closed tab does nothing",
        group(B, q("First?", "x", "y"), q("Second?", "yes", "no")),
        seq(poll(close(1, "answered")), keys("enter", "1", "enter")), focus="w1:p2")
    add("race.several_to_one_does_not_make_enter_send", "a group down to one question still drafts",
        group(A, q("Stays?", "a", "b"), q("Goes?", "c")), seq(poll(close(2)), keys("1", "enter")))
    add("race.new_question_mid_typing_keeps_the_text", "other tabs coming and going never touch the view",
        ask(A, "Only question?", options=["a", "b"]),
        seq(keys("1", "space", "n"), typed("note"), keys("enter", "3", "enter"), typed("half typed"),
            poll(ask(B, "Second?", options=["x"])), poll(close(2))))
    add("race.go_to_agent_works_for_multi_select", "g on a multi-select question",
        ask(A, "Which?", options=["x", "y"], multi_select=True), seq(keys("g"), focused("w1:p1")))

    def three_with_failed(store):
        a = service.ask(store, A, "One?")["id"]
        b = service.ask(store, B, "Two?")["id"]
        service.ask(store, A, "Three?")
        with env(FAKE_HERDR_FAIL="agent prompt"):
            service.answer(store, a, "first")
            service.answer(store, b, "second")

    add("race.failed_delivery_entry_retry_and_hide", "⚠ entries: Retry sends, Stop showing hides",
        three_with_failed, seq(keys("2", "enter", "1"), sent(), keys("3", "enter", "2", "up")))

    # -- tests/test_real_timing.py (keys 150 ms apart, the real windows) --------------------
    add("real_timing.list_wrap_with_real_arming", "↓ Enter Enter Enter right after an arrival never answers it",
        setup(ask(A, "q1?", options=["a"]), ask(B, "q2?", options=["b"])),
        seq(keys("down"), poll(ask(B, "drop prod?", options=["yes"])), keys("down", "enter", "enter", "enter", gap=150)))
    add("real_timing.list_row_that_just_arrived_is_not_selectable", "a fresh row: no digit, no Enter for 1 s",
        setup(ask(A, "q1?", options=["a"]), ask(B, "q2?", options=["b"])),
        seq(keys("down"), poll(ask(B, "fresh?", options=["f"])), keys("3", "down", "enter", gap=150),
            key("enter", gap=700)))
    add("real_timing.question_view_ignores_keys_until_armed", "keys right behind the open are ignored",
        setup(ask(A, "q1?", options=["a", "b"]), ask(B, "q2?", options=["c"])),
        seq(keys("enter"), fast("2", "enter"), keys("2", "enter")))
    add("real_timing.submit_tab_ignores_keys_until_armed", "the Submit tab arms too",
        group(A, q("q1?", "a"), q("q2?", "b")), seq(keys("tab", "tab"), keys("enter", gap=150), keys("down", "enter")))

    def late_and_other(store):
        failed(store, A, "late?", "y", options=["y"])
        service.ask(store, B, "other?")

    add("real_timing.undelivered_view_ignores_keys_until_armed", "a ⚠ entry's view arms too",
        late_and_other, seq(keys("down", "enter"), keys("1", gap=150), keys("1"), sent()), focus="w1:p2")

    # -- tests/test_alerts.py ---------------------------------------------------------------
    def two_and_alert(store):
        service.ask(store, B, "A question?")
        service.ask(store, B, "Another?")
        alerts.raise_alert(store, A, "Disk almost full", "92% on /data", "urgent")

    add("alerts.popup_shows_alerts_first_unnumbered_and_d_acknowledges", "alerts head the list; no Enter action",
        two_and_alert, keys("up", "enter", "enter", "1", "2", "d"), focus="w1:p2")
    add("alerts.popup_d_acknowledges_an_alert_and_g_goes_to_the_pane", "d on the alert row acknowledges it",
        two_and_alert, keys("up", "d", "down", "up"), focus="w1:p2")
    add("alerts.popup_g_goes_to_the_alerts_pane", "g on the alert row after a pause",
        two_and_alert, seq(keys("up", "g"), focused("w1:p1")), focus="w1:p2")
    add("alerts.an_alert_withdrawn_while_on_screen_becomes_inert", "a withdrawn alert's view: a placeholder",
        two_and_alert, seq(keys("up", "enter"), poll(lambda s: alerts.withdraw(s, A, 1)), keys("1", "enter", "escape")),
        focus="w1:p2")
    add("alerts.m1_enter_enter_never_goes_to_a_blocked_agent", "Enter, Enter: the question, as before 0.8",
        setup(ask(B, "Merge it?", options=["yes", "no"]), blocked_alert), keys("enter"), focus="w1:p2")
    add("alerts.m1_a_blocked_alert_view_goes_only_on_a_deliberate_g", "g right after a key goes nowhere",
        setup(blocked_alert, ask(B, "Q?"), ask(B, "Q2?")),
        seq(keys("up"), fast("g"), keys("enter"), fast("enter", "1", "g"), keys("g"), focused("w1:p1")), focus="w1:p2")
    add("alerts.h1_habitual_sequences_with_an_alert_open", "1 Enter 1 Enter: the same as without the alert",
        setup(ask(B, "Merge it?", options=["yes", "no"]), blocked_alert), keys("1", "enter"),
        focus="w1:p2")
    add("alerts.h1_habitual_1_g_with_an_alert_open", "1 g: goes to the question's own agent",
        setup(ask(B, "Merge it?", options=["yes", "no"]), blocked_alert), seq(keys("1", "g"), focused("w1:p2")),
        focus="w1:p2")
    add("alerts.h1_the_only_question_still_opens_directly_with_an_alert_open", "an alert never changes the start",
        setup(ask(B, "Only one?", options=["yes", "no"]), lambda s: alerts.raise_alert(s, A, "Check it")), [],
        focus="w1:p2")

    def g_presses(name, gaps):
        add(f"alerts.{name}", "g presses on an alert view: only one after a 0.5 s pause goes",
            setup(ask(B, "Q?"), ask(B, "Q2?"), lambda s: alerts.raise_alert(s, A, "Check the deploy")),
            seq(keys("up", "enter"), [key("g", gap=g) for g in gaps]), focus="w1:p2")

    g_presses("a_held_g_never_passes_the_pause", [80] * 8)
    g_presses("a_double_tapped_g_never_passes_the_pause", [200, 350])
    g_presses("a_deliberate_g_after_a_pause_still_goes", [100, 600])

    # -- tests/test_failed_pile.py, tests/test_agent_gone.py --------------------------------
    D = Caller("w1:p2", "w1:t2", "tab-B", "bbbbbbbb-0000-4000-8000-00000000000b", "/r")

    def gone_rows(n, live=True):
        def run(store):
            for i in range(n):
                gone(store, A, f"Gone question {i}?")
            if live:
                service.ask(store, D, "A live question?")
        return run

    add("failed_pile.q167_gone_answers_collapse_into_one_row_and_d_hides_them_all", "one row; d hides them all",
        gone_rows(3), keys("down", "d", "enter", "up"), focus="w1:p2")
    add("failed_pile.q167_a_single_gone_answer_keeps_its_own_row", "a lone gone answer: its own row; d hides it",
        gone_rows(1), keys("down", "enter", "escape", "d"), focus="w1:p2")
    add("failed_pile.q167_a_gone_answer_arriving_later_never_joins_the_open_summary", "the summary is fixed at open",
        gone_rows(2), seq(poll(lambda s: gone(s, A, "Late gone?")), keys("down", "d", "down", "d")), focus="w1:p2")

    def busy_and_other(store):
        qid = service.ask(store, A, "Busy?", options=["yes"])["id"]
        with env(FAKE_HERDR_STATUS="working"):
            service.answer(store, qid, "yes")
        service.ask(store, D, "Other?")

    add("agent_gone.pending_rows_say_they_are_already_answered", "a pending answer opens its ⚠ view; d does nothing",
        busy_and_other, keys("down", "d", "enter", "down", "escape"), focus="w1:p2")

    # -- tests/test_deep_link.py ------------------------------------------------------------
    def three_groups(store):
        service.ask_group(store, A, [q("a1?"), q("a2?")])
        service.ask_group(store, D, [q("b1?"), q("b2?"), q("b3?")])
        service.ask(store, D, "b-later?")

    question4 = {"target": {"kind": "question", "value": "4"}, "resolved": (("group", 3), 4)}
    add("deep_link.popup_opens_on_the_requested_question", "held: nothing but Esc until a deliberate Tab",
        three_groups, seq(fast("1", "enter"), keys("escape")), deeplink=question4)
    add("deep_link.popup_opens_on_the_group_for_each_target_kind", "a group target opens on its first blank",
        three_groups, [], deeplink={"target": {"kind": "group", "value": "3"}, "resolved": (("group", 3), None)})
    add("deep_link.gone_target_falls_back_to_the_list_with_a_notice", "the target is gone: the normal start",
        three_groups, [], deeplink={"target": {"kind": "question", "value": "9"}, "resolved": None})

    solo = {"target": {"kind": "question", "value": "1"}, "resolved": (("group", 1), 1)}
    add("deep_link.a_deep_linked_view_waits_for_a_deliberate_key", "only Tab/→ after 0.5 s releases; then nothing else",
        ask(A, "solo?", options=["yes", "no"]),
        seq(keys("enter", "1", "2", "enter", "up", "enter", "down", "j", "k", "enter", "tab", "enter", gap=30),
            keys("right", "enter")), focus="w1:p2", deeplink=solo)
    add("deep_link.no_hold_opens_like_a_normal_popup", "--no-hold: armed as usual, not held",
        ask(A, "solo?", options=["yes", "no"]), seq(fast("enter"), keys("enter")), focus="w1:p2",
        deeplink={**solo, "hold": False})

    # -- edge cases the model must match ----------------------------------------------------
    add("edge.note_box_keys", "inside a note: arrows, digits, d, g and Tab are text or nothing",
        group(A, q("One?", "a", "b"), q("Two?", "c")),
        seq(keys("n"), typed("ab"), keys("left", "left", "x", "right", "backspace", "home", "end", "up", "down",
                                         "1", "d", "g", "tab")))
    add("edge.confirm_under_tab", "a confirmation open: Tab and keys",
        group(A, q("One?", "a"), q("Two?", "b")), keys("d", "tab", "y", "d", "tab", "d", "left", "d", "y"))
    add("edge.empty_view_keys", "the empty view with arrivals: arrows and Tab open the list, others close",
        lambda s: None, seq(poll(ask(A, "late?")), keys("left", "escape")))
    add("edge.empty_view_up_closes", "the empty view with an arrival: ↑ is any key, it closes",
        lambda s: None, seq(poll(ask(A, "late?")), keys("up")))
    add("edge.review_keys", "the Submit tab: digits, x drops a draft, Edit goes to the question",
        group(A, q("One?", "a"), q("Two?", "b")),
        seq(keys("enter", "enter", "up", "up", "x", "down", "enter", "tab", "1"), sent()))
    add("edge.list_digits_skip_alerts", "digits count question rows only; ↓ on the last row stays",
        setup(lambda s: alerts.raise_alert(s, A, "Look"), ask(B, "Q?"), ask(B, "Q2?")),
        keys("2", "1", "down", "down", "down", "up", "up", "up", "9"), focus="w1:p2")
    add("edge.closed_group_poll_inside_list", "rows close and arrive while the list is shown",
        setup(ask(A, "one?"), ask(B, "two?"), ask(C, "three?")),
        seq(poll(setup(close(1, "answered"), ask(A, "four?"))), keys("enter", "down", "down")))
    add("edge.tab_wraps_and_switch_in_list", "←/→ in the list do nothing; in a group they wrap",
        setup(group(A, q("One?"), q("Two?")), ask(B, "Other?")),
        keys("left", "right", "enter", "left", "left", "right", "shift+tab"))
    add("edge.tab_in_the_list", "Tab in the list (no tabs there): what the next keys do",
        setup(ask(A, "one?", options=["a"]), ask(B, "two?", options=["b"])), keys("tab", "down", "enter", "1", "enter"))
    add("edge.tab_in_a_failed_view", "Tab on a ⚠ entry's view",
        late_and_other, keys("down", "enter", "tab", "down", "1", "escape"), focus="w1:p2")
    add("edge.tab_in_an_alert_view", "Tab on an alert's view",
        two_and_alert, keys("up", "enter", "tab", "d", "escape"), focus="w1:p2")
    add("edge.tab_in_the_empty_view", "Tab with nothing open: any key closes", lambda s: None, keys("tab"))
    return out
