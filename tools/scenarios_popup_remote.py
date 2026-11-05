"""The remote-machine scenarios of the popup transition golden (P6-D2), after
tools/scenarios_popup.py's: a remote machine's open groups (``remote:<machine>|<db>|<group>``),
its queued or failed actions (``routbox:<request id>``), the answers it cannot deliver
(``rproblem:<machine>|<id>``) and its alerts (``ralert:<machine>|<id>``). ``remote_sync.*``
replay tests/test_remote_sync.py's popup tests; ``remote_edge.*`` pin down what no test
asserts but a model must still match.

The mirror is written directly (``popup_remote``'s helpers): machine ``m1`` labelled
``project1``; remote questions are asked from its tab-A (pane ``w1:p1`` over there).
"""

from __future__ import annotations

import json


def REMOTE_SCENARIOS(h):  # noqa: N802, C901 - the list the capture runs
    import popup_remote as r

    from herdr_inbox import alerts, service

    A, B, C = h.callers()
    key, keys, typed, poll, sent, focused, unseen = h.key, h.keys, h.typed, h.poll, h.sent, h.focused, h.unseen
    out = []

    def add(name, doc, setup, steps, focus="w1:p2", deeplink=None):
        out.append((h.Run(name, doc, setup, focus, deeplink), list(steps)))

    def seq(*parts):
        steps = []
        for p in parts:
            steps += p if isinstance(p, list) else [p]
        return steps

    def setup(*fns):
        def run(store):
            for fn in fns:
                fn(store)
        return run

    def local(caller=B, text="Local?", **kw):
        return lambda s: service.ask(s, caller, text, **kw)

    def machine(**kw):
        return lambda s: r.machine(s, **kw)

    def rq(rid, text, *options, **kw):
        return lambda s: r.rq(s, rid, text, *options, **kw)

    def ralert(aid, title, **kw):
        return lambda s: r.ralert(s, aid, title, **kw)

    def outbox(rid, action, **kw):
        return lambda s: r.outbox(s, rid, action, **kw)

    def problem(rid, text):
        return lambda s: r.rq(s, rid, text, "yes", status="answered", answer="yes", agent_gone=True)

    def closed_there(rid, status="withdrawn"):
        """Answered, withdrawn or dismissed over there: the next pull mirrors its new status."""
        def run(s):
            d = r.shown(s, rid)
            d.pop("_db_id")
            s.db.execute("UPDATE remote_questions SET status = ?, data_json = ? WHERE machine_id = ? AND remote_id = ?",
                         (status, json.dumps({**d, "status": status}), r.MID, rid))
        return run

    def replaced(rid, text):
        """The remote inbox was reset: a new question under the same number (other db_id, qkey)."""
        return lambda s: r.rq(s, rid, text, "yes", dbid="e" * 32, qkey="b" * 32)

    one = setup(machine(), rq(1, "Ship it?", "yes", "no"), local())

    # -- tests/test_remote_sync.py ------------------------------------------------------------
    add("remote_sync.popup_lists_and_answers_a_remote_question",
        "a remote group is a list row; Enter opens its question, a pick is queued for the worker",
        one, seq(keys("down", "enter", "enter"), poll()))
    add("remote_sync.popup_dismisses_a_remote_group_with_confirmation", "d asks; y queues a dismiss",
        setup(machine(), rq(1, "Drop?"), local()), seq(keys("down", "d", "y"), poll()))
    add("remote_sync.popup_shows_an_offline_machine_and_a_rejection", "d clears a rejected action",
        setup(machine(offline=True), rq(1, "Q?", "yes"), local(),
              outbox(1, "hide", state="rejected", result="answered with no")),
        seq(keys("down", "down", "d"), poll()))
    add("remote_sync.deep_link_to_a_remote_question", "a deep link opens the remote question (no hold)",
        setup(machine(), rq(1, "Remote?", "yes")),
        keys("enter"), focus="w9:p9",
        deeplink={"target": {"kind": "question", "value": "project1:Q1"}, "hold": False,
                  "resolved": (("remote", f"m1|{r.DBID}|1"), 1)})
    add("remote_sync.deep_link_to_a_remote_question.held",
        "held: no answering key, and Tab/→ never release it in a remote question (no tabs)",
        setup(machine(), rq(1, "Remote?", "yes")),
        keys("1", "tab", "right", "enter", "escape", "enter"), focus="w9:p9",
        deeplink={"target": {"kind": "question", "value": "project1:Q1"},
                  "resolved": (("remote", f"m1|{r.DBID}|1"), 1)})
    add("remote_sync.a_remote_question_closed_there_becomes_an_inert_placeholder",
        "closed on the remote while on screen: an inert placeholder; Enter queues nothing",
        setup(machine(), rq(1, "Ship?", "yes"), local()),
        seq(keys("down", "enter"), poll(closed_there(1)), keys("enter", "escape")))
    add("remote_sync.m1_the_popup_shows_a_past_expiry_as_expired", "d on an expired queued action settles and clears it",
        setup(machine(), rq(1, "Q?", "yes"), local(), outbox(1, "answer", expired=True)),
        seq(keys("down", "d"), poll()))
    add("remote_sync.m2_the_popup_lists_it_and_d_clears_it", "an answer the remote cannot deliver: d clears it here",
        setup(machine(), problem(1, "Q?"), local()), seq(keys("down", "d"), poll()))
    add("remote_sync.popup_acknowledges_a_remote_alert", "a remote alert opens with its machine; g never goes; d acks",
        setup(machine(), ralert(1, "[E2E TEST] remote alert"), local(), local(text="Local 2?")),
        seq(keys("up", "enter", "g", "d"), poll()))
    add("remote_sync.m1_an_answer_to_the_question_on_screen_never_lands_on_a_replacement",
        "the mirror's question was replaced under the view (no poll): Enter queues nothing",
        one, seq(keys("down", "enter"), unseen(replaced(1, "Deploy to prod?")), keys("enter", "escape")))

    # -- edges ---------------------------------------------------------------------------------
    add("remote_edge.only_remote_rows", "nothing local: the list opens, the cursor on the first remote row",
        setup(machine(), rq(1, "Ship it?", "yes")), keys("enter", "1", "escape"))
    add("remote_edge.answering_one_of_two", "a group of two: the first answered, the row stays and opens the second",
        setup(machine(), rq(1, "First?", "a", group=1), rq(2, "Second?", "b", group=1), local()),
        seq(keys("down", "enter", "enter", "enter", "1", "enter"), poll()))
    add("remote_edge.last_remote_answer_closes", "the only row answered: the popup quits",
        setup(machine(), rq(1, "Ship it?", "yes")), keys("enter", "1", "enter"))
    add("remote_edge.enter_and_g_on_remote_rows", "Enter on an outbox/problem row only says so; g never goes",
        setup(machine(), ralert(1, "Look"), rq(1, "Q?", "yes"), rq(2, "Q2?", "yes"), problem(3, "Gone?"),
              outbox(1, "answer"), local()),
        keys("up", "g", "down", "down", "g", "down", "g", "enter", "d", "down", "g", "enter", "up", "up", "d"))
    add("remote_edge.remote_view_keys", "in a remote question: no tabs, no d, no g; x, a note, Tab out of the box",
        setup(machine(), rq(1, "Which?", "a", "b", header="Pick"), local()),
        seq(keys("down", "enter", "right", "left", "d", "g", "x", "down", "n"), typed("hi"),
            keys("tab", "tab", "escape")))
    add("remote_edge.chat_about_this", "Chat about this on a remote question: Python focuses that pane id here",
        setup(machine(), rq(1, "Which?", "a"), local()), seq(keys("down", "enter", "3", "enter"), focused("w1:p1")))
    add("remote_edge.multi_select_and_empty_text", "multi-select needs a pick; empty typed text answers nothing",
        setup(machine(), rq(1, "Which?", "a", "b", multi_select=True), rq(2, "Free?"), local()),
        seq(keys("down", "enter", "up", "enter", "up", "up", "up", "space", "down", "down", "down", "enter"),
            keys("down", "enter", "enter", "enter", "enter"), typed("ok"), keys("enter")))
    add("remote_edge.arrivals_are_armed", "a remote row that arrives takes no digit and no Enter for a moment",
        setup(machine(), local(), local(A, "Mine?")),
        seq(poll(rq(1, "Late?", "yes"), gap=500), key("3", gap=100), key("down", gap=100), key("down", gap=100),
            key("enter", gap=100), keys("enter")))
    add("remote_edge.remote_rows_end", "remote rows ending become placeholders with their reasons",
        setup(machine(), rq(1, "Q?", "yes"), problem(2, "Gone?"), ralert(1, "Look"), outbox(1, "hide", state="failed"),
              local()),
        seq(poll(lambda s: (s.db.execute("DELETE FROM remote_questions"), s.db.execute("DELETE FROM remote_alerts"),
                            s.db.execute("UPDATE remote_outbox SET state = 'seen'"))), keys("down")))
    add("remote_edge.remote_alert_ends_on_screen", "a remote alert withdrawn on screen: its placeholder names the machine",
        setup(machine(), ralert(1, "Look"), local(), local(text="Two?")),
        seq(keys("up", "enter"), poll(ralert(1, "Look", status="withdrawn")), keys("d", "escape")))
    add("remote_edge.remote_alert_d_in_the_list", "d on a remote alert row acknowledges it",
        setup(machine(), ralert(1, "Look"), local(), local(text="Two?")), seq(keys("up", "d", "down"), poll()))
    add("remote_edge.queued_row_is_not_cleared", "d on a still-queued (or being sent) action does nothing",
        setup(machine(), rq(1, "Q?", "yes"), rq(2, "Q2?", "yes"), outbox(1, "answer"), outbox(2, "dismiss", expired=True,
                                                                                           sending=True), local()),
        keys("down", "d", "down", "d", "escape"))
    add("remote_edge.dismiss_kept_and_polled_under", "n keeps the remote group; a poll waits for the confirmation",
        setup(machine(), rq(1, "First?", group=1), rq(2, "Second?", group=1), local()),
        seq(keys("down", "d", "n", "d"), poll(rq(3, "Late?")), keys("y"), poll()))
    add("remote_edge.unlabelled_machine", "a machine without a label: its id stands in",
        setup(machine(label=None), rq(1, "Q?", "yes"), ralert(1, "Look"), local()),
        seq(keys("down", "enter", "enter", "up", "up", "enter"), poll(ralert(1, "Look", status="withdrawn"))))
    add("remote_edge.remote_question_ends_after_answer_elsewhere", "a remote question answered there: placeholder",
        setup(machine(), rq(1, "Q?", "yes"), local()),
        seq(keys("down", "enter"), poll(closed_there(1, "answered")), keys("1", "right", "escape", "escape")))

    # -- the review's probes (#130): what guards the wrong question, machine or pane -------------
    pv = [{"label": "a", "preview": "PA"}, {"label": "b", "preview": "PB"}]
    two = setup(machine(), rq(1, "First?", "a", group=1), rq(2, "Second?", "b", group=1), local())
    ab = setup(machine(), rq(1, "Which?", "a", "b"), local())
    ap = setup(machine(), rq(1, "Which?", options=pv), local())
    T, S = "tab", "shift+tab"
    add("remote_edge.closed_but_group_lives", "the question on screen closes, its group lives: still a placeholder",
        two, seq(keys("down", "enter"), poll(closed_there(1)), keys("enter", "escape", "down", "enter")))
    add("remote_edge.two_machines_same_id", "an answer on one machine leaves the other's question of that number",
        setup(machine(), machine(mid="m2", label="project2"), rq(1, "Mine?", "a"), rq(1, "Theirs?", "b", mid="m2"),
              local()),
        seq(keys("down", "enter", "enter", "down", "enter", "enter"), poll()))
    add("remote_edge.tab_chain_box", "Tab and Shift+Tab with a box open: view, options, box",
        ab, seq(keys("down", "enter", "n"), typed("hi"), keys(T, T, T, T, S, S, S, S, "escape")))
    add("remote_edge.tab_chain_preview", "the chain takes in the preview pane", ap, keys("down", "enter", T, T, T, S, S, S))
    add("remote_edge.tab_chain_preview_box", "the whole chain: view, options, box, preview",
        ap, seq(keys("down", "enter", "n"), typed("hi"), keys(T, T, T, T, T, S, S, S, S, S, "escape")))
    add("remote_edge.tab_chain_shared_id", "a local question of that number has a preview, the remote one none",
        setup(machine(), rq(1, "Which?", "a"), lambda s: service.ask(s, B, "Local?", options=pv)),
        keys("down", "enter", T, T, T))
    add("remote_edge.tab_choices_then_pick", "keys reach the view from the option list",
        ab, keys("down", "enter", T, "down", "enter"))
    add("remote_edge.two_alerts_only", "only remote alerts: an ack goes back to the list, the last one closes",
        setup(machine(), ralert(1, "One"), ralert(2, "Two")), seq(keys("down", "enter", "d", "down", "enter"), poll()))
    add("remote_edge.g_in_a_remote_alert_shares_a_local_id", "g on a remote alert never goes to the local alert of that number",
        setup(machine(), ralert(1, "Theirs"), lambda s: alerts.raise_alert(s, B, "Ours", "body", "urgent"), local(),
              local(text="Two?")),
        keys("up", "enter", "g", "g"))
    return out
