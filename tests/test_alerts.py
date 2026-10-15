"""Alerts (0.8): an agent's "please check X", and agents stuck at a blocking prompt."""

import json

import pytest

from herdr_inbox import alerts, service, tui, worker
from herdr_inbox.service import Caller
from herdr_inbox.tui import AlertView, GroupListView, InboxApp

@pytest.fixture(autouse=True)
def _fresh_scan(monkeypatch):
    """scan_blocked's per-process memory, fresh per test; hysteresis by ticks only."""
    alerts._unblocked.clear()
    monkeypatch.setattr(alerts, "CLEAR_AFTER", 0.0)
    monkeypatch.setattr(tui, "ALERT_GO_GRACE", 0.0)


A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")
B = Caller("w1:p2", "w1:t2", "tab-B", "sess-B", "/r")


def test_raise_notifies_counts_in_the_badge_and_dedupes(store, fake_herdr):
    r = alerts.raise_alert(store, A, "Deploy needs your eyes", "prod rollout paused at 50%", "urgent", "deployer")
    assert r["status"] == "open" and r["id"] == 1 and "no answer comes back" in r["message"]
    show = fake_herdr.calls_of("notification", "show")[-1]
    assert show[2] == "A1 · urgent · tab-A" and show[4] == "Deploy needs your eyes"
    assert store.open_counts() == {"w1:p1": 1}
    service.ask(store, A, "And a question?")
    assert store.open_counts() == {"w1:p1": 2}
    assert ["--token", "inbox_q=?2"] == fake_herdr.calls_of("pane", "report-metadata")[-1][-2:]
    assert alerts.raise_alert(store, A, "Deploy needs your eyes")["status"] == "already_open"


def test_bad_input_and_outside_herdr(store, fake_herdr):
    assert alerts.raise_alert(store, A, "  ")["status"] == "error"
    assert alerts.raise_alert(store, Caller(None, None, None, None, None), "x")["status"] == "unavailable"
    r = alerts.raise_alert(store, A, "x" * 500, severity="nonsense")
    a = alerts.get(store, r["id"])
    assert len(a.title) == alerts.MAX_TITLE and a.severity == "warning"


def test_withdraw_and_acknowledge(store, fake_herdr):
    aid = alerts.raise_alert(store, A, "Check the logs")["id"]
    assert alerts.withdraw(store, B, aid)["status"] == "error"  # not that session's
    assert alerts.withdraw(store, A, aid)["status"] == "withdrawn"
    assert store.open_counts() == {}
    aid = alerts.raise_alert(store, A, "Check again")["id"]
    assert alerts.acknowledge(store, aid) and not alerts.acknowledge(store, aid)
    assert [a.id for a in alerts.acknowledged_unreported(store, A)] == [aid]
    assert alerts.acknowledged_unreported(store, A) == []  # reported once


async def test_mcp_tools(store, fake_herdr):
    from mcp import Client

    from herdr_inbox.server import mcp

    async with Client(mcp) as client:
        r = (await client.call_tool("alert_operator", {"title": "Please check the build", "severity": "info"})).structured_content
        assert r["status"] == "open"
        aid = r["id"]
        alerts.acknowledge(store, aid)
        r = (await client.call_tool("check_answers", {})).structured_content
        assert r["alerts_acknowledged"] == [{"id": f"A{aid}", "title": "Please check the build"}]
        r = (await client.call_tool("list_my_questions", {"include_closed": True})).structured_content
        assert r["alerts"][0]["status"] == "acked"
        aid2 = (await client.call_tool("alert_operator", {"title": "Other"})).structured_content["id"]
        r = (await client.call_tool("withdraw_alert", {"id": aid2})).structured_content
        assert r["status"] == "withdrawn"


async def test_popup_shows_alerts_first_unnumbered_and_d_acknowledges(store, fake_herdr):
    service.ask(store, B, "A question?")
    service.ask(store, B, "Another?")
    aid = alerts.raise_alert(store, A, "Disk almost full", "92% on /data", "urgent")["id"]
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        # listed first, unnumbered, cursor not on it; the questions keep 1, 2
        assert body.startswith(f"     ⚑ A{aid} · urgent · Disk almost full")
        assert "❯ 1. A question?" in body and "  2. Another?" in body
        await pilot.press("up", "enter")
        await pilot.pause()
        assert app.mode == "alert" and "92% on /data" in str(app.query_one(AlertView).query(".question")[1].render())
        await pilot.press("enter", "1", "2")  # no Enter action, no numbered options
        await pilot.pause()
        assert app.mode == "alert" and alerts.get(store, aid).status == "open"
        await pilot.press("d")
        await pilot.pause()
        assert app.mode == "list"
    assert alerts.get(store, aid).status == "acked"
    assert not fake_herdr.calls_of("agent", "prompt")  # nothing is typed back


async def test_popup_d_acknowledges_an_alert_and_g_goes_to_the_pane(store, fake_herdr):
    service.ask(store, B, "Q?")
    service.ask(store, B, "Q2?")
    a1 = alerts.raise_alert(store, A, "One")["id"]
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("up", "d")
        await pilot.pause()
    assert alerts.get(store, a1).status == "acked"
    alerts.raise_alert(store, A, "Two")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("up", "g")
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert fake_herdr.calls_of("agent", "focus") or fake_herdr.calls_of("pane", "focus")


def blocked(pane="w1:p1", status="blocked"):
    return json.dumps([{"pane_id": pane, "tab_id": "w1:t1", "agent": "claude", "agent_status": status,
                        "agent_session": {"value": "sess-A"}}])


def test_an_agent_stuck_at_a_prompt_becomes_an_alert_and_clears_itself(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 0.0)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())
    worker.tick(store)
    worker.tick(store)
    (a,) = alerts.open_alerts(store)
    assert a.kind == "blocked" and a.title == "my-tab is waiting for you"
    assert len([c for c in fake_herdr.calls_of("notification", "show") if "waiting" in c[4]]) == 1
    assert store.open_counts() == {"w1:p1": 1}
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked(status="working"))
    worker.tick(store)
    assert alerts.open_alerts(store) != []  # one tick of another status is not enough (flicker)
    worker.tick(store)
    assert alerts.open_alerts(store) == [] and alerts.get(store, a.id).status == "cleared"
    assert not fake_herdr.calls_of("agent", "prompt") and not fake_herdr.calls_of("pane", "send-keys")


def test_a_short_prompt_never_raises_an_alert(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 3600.0)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())
    worker.tick(store)
    assert alerts.open_alerts(store) == []


def test_markup_escape_keeps_upper_case_brackets_and_trailing_backslashes():
    from textual.markup import to_content

    from herdr_inbox.tui import escape

    for text in ("[E2E TEST] check", "a [b] [/] [not", "ends with \\", "x\\[y]"):
        assert to_content(f"[b]{escape(text)}[/b] tail").plain.replace("\u200b", "") == text + " tail"


def test_the_worker_stays_alive_while_herdr_reports_agents_and_exits_without(fake_herdr, monkeypatch):
    from herdr_inbox import remote

    monkeypatch.setattr(worker, "IDLE_EXIT", 0.0)
    monkeypatch.setattr(worker, "TICK", 0.0)
    monkeypatch.setattr(remote, "enabled_machines", lambda ttl=60.0: [])
    ticks = {"n": 0}

    def tick(store):
        ticks["n"] += 1
        worker.watching = 1 if ticks["n"] < 4 else 0
        return 0

    monkeypatch.setattr(worker, "tick", tick)
    assert worker.run() == 0
    assert ticks["n"] == 4  # kept alive while agents were watched, then idle-exited


def test_tick_counts_the_agents_it_watches(store, fake_herdr, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked(status="idle"))
    worker.tick(store)
    assert worker.watching == 1
    monkeypatch.setenv("FAKE_HERDR_AGENTS", "[]")
    worker.tick(store)
    assert worker.watching == 0


async def test_an_alert_withdrawn_while_on_screen_becomes_inert(store, fake_herdr):
    service.ask(store, B, "Q?")
    service.ask(store, B, "Q2?")
    aid = alerts.raise_alert(store, A, "Look")["id"]
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("up", "enter")
        await pilot.pause()
        assert app.mode == "alert"
        alerts.withdraw(store, A, aid)
        app.poll()
        await pilot.pause()
        from herdr_inbox.tui import GoneView

        assert app.query_one(GoneView).ref == f"A{aid}"
        await pilot.press("1", "enter")
        await pilot.pause()
        assert app.mode == "alert"  # nothing selected, nothing moved
    assert alerts.get(store, aid).status == "withdrawn"


def test_a_dismissed_blocked_alert_holds_until_the_prompt_ends(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 0.0)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())
    worker.tick(store)
    (a,) = alerts.open_alerts(store)
    alerts.acknowledge(store, a.id)  # the operator dismissed it; the prompt is still up
    worker.tick(store)
    worker.tick(store)
    assert alerts.open_alerts(store) == []
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked(status="working"))
    worker.tick(store)
    worker.tick(store)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())  # the next prompt alerts again
    worker.tick(store)
    assert len(alerts.open_alerts(store)) == 1


def test_l_a_flickering_status_neither_clears_nor_renotifies(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 0.0)
    monkeypatch.setattr(alerts, "CLEAR_AFTER", 3600.0)
    for status in ("blocked", "blocked", "working", "working", "working", "blocked", "working", "blocked"):
        monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked(status=status))
        worker.tick(store)
    (a,) = alerts.open_alerts(store)
    assert a.id == 1 and len([c for c in fake_herdr.calls_of("notification", "show") if "waiting" in c[4]]) == 1


def test_l_an_agent_gone_from_the_list_clears_at_once(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 0.0)
    monkeypatch.setattr(alerts, "CLEAR_AFTER", 3600.0)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())
    worker.tick(store)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", "[]")
    worker.tick(store)
    assert alerts.open_alerts(store) == []


def test_l_another_herdr_servers_alert_for_the_same_pane_id_does_not_suppress(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 0.0)
    for status in ("open", "acked"):
        store.db.execute("INSERT INTO alerts (created_at, kind, severity, title, pane_id, herdr_server, status)"
                         " VALUES ('2026-10-15T00:00:00+00:00', 'blocked', 'warning', 'x', 'w1:p1', '/other.sock', ?)",
                         (status,))
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())
    worker.tick(store)
    mine = [a for a in alerts.open_alerts(store) if a.herdr_server != "/other.sock"]
    assert len(mine) == 1 and mine[0].pane_id == "w1:p1"


async def _enter_enter(store, fake_herdr, kind):
    service.ask(store, B, "Merge it?", options=["yes", "no"])
    if kind == "blocked":
        store.db.execute("INSERT INTO alerts (created_at, kind, severity, title, pane_id, tab_label, status)"
                         " VALUES (?, 'blocked', 'warning', 'tab-A is waiting for you', 'w1:p1', 'tab-A', 'open')",
                         (alerts.now(),))
    else:
        alerts.raise_alert(store, A, "Check the deploy log")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("enter", "enter")
        await pilot.pause()
        exited = app.return_code is not None or not app.is_running
        mode = app.mode
    return app, exited, mode


async def test_m1_enter_enter_never_goes_to_a_blocked_agent(store, fake_herdr):
    app, exited, mode = await _enter_enter(store, fake_herdr, "blocked")
    assert not fake_herdr.calls_of("agent", "focus") and not fake_herdr.calls_of("pane", "focus")
    assert alerts.open_alerts(store)[0].status == "open"
    assert mode != "alert"  # Enter, Enter went to the question, as before 0.8


async def test_m1_enter_enter_never_acknowledges_an_agent_alert(store, fake_herdr):
    await _enter_enter(store, fake_herdr, "agent")
    assert [a.status for a in alerts.open_alerts(store)] == ["open"]


async def test_m1_a_blocked_alert_view_goes_only_on_a_deliberate_g(store, fake_herdr, monkeypatch):
    store.db.execute("INSERT INTO alerts (created_at, kind, severity, title, pane_id, tab_label, status)"
                     " VALUES (?, 'blocked', 'warning', 'tab-A is waiting for you', 'w1:p1', 'tab-A', 'open')",
                     (alerts.now(),))
    service.ask(store, B, "Q?")
    service.ask(store, B, "Q2?")
    monkeypatch.setattr(tui, "ALERT_GO_GRACE", 3600.0)
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("up", "g")  # list g on the alert row, right after a move: refused
        await pilot.press("enter")
        await pilot.pause()
        assert app.mode == "alert"
        await pilot.press("enter", "1", "g")  # nothing on Enter or a digit; g without a pause: refused
        await pilot.pause()
        await app.workers.wait_for_complete()
        assert not fake_herdr.calls_of("agent", "focus") and not fake_herdr.calls_of("pane", "focus")
        monkeypatch.setattr(tui, "ALERT_GO_GRACE", 0.0)
        await pilot.press("g")
        await pilot.pause()
        await app.workers.wait_for_complete()
    assert fake_herdr.calls_of("agent", "focus") or fake_herdr.calls_of("pane", "focus")


async def _sequence(store, fake_herdr, keys, with_alert):
    """The reviewer's habitual sequences at a human pace (0.6 s apart), with the real grace."""
    import asyncio

    service.ask(store, B, "Merge it?", options=["yes", "no"])
    if with_alert:
        store.db.execute("INSERT INTO alerts (created_at, kind, severity, title, pane_id, tab_label, status)"
                         " VALUES (?, 'blocked', 'warning', 'tab-A is waiting for you', 'w1:p1', 'tab-A', 'open')",
                         (alerts.now(),))
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        for key in keys:
            await asyncio.sleep(0.6)
            await pilot.press(key)
            await pilot.pause()
        await app.workers.wait_for_complete()
        await pilot.pause()
    q = store.get(1)
    return (q.status, q.answer), [c[2] for c in fake_herdr.calls_of("agent", "focus")]


@pytest.mark.parametrize("keys", [("1", "enter", "1"), ("1", "g"), ("1", "enter", "1", "enter")])
async def test_h1_habitual_sequences_with_an_alert_open_behave_as_in_0_7(tmp_path, fake_herdr, monkeypatch, keys):
    from herdr_inbox.store import Store

    monkeypatch.setattr(tui, "ALERT_GO_GRACE", 0.5)
    results = []
    for with_alert in (False, True):
        fake_herdr.reset()
        with Store(tmp_path / f"{with_alert}.db") as store:
            results.append(await _sequence(store, fake_herdr, keys, with_alert))
    assert results[0] == results[1]  # the same answer and the same focus, alert or not
    assert "w1:p1" not in results[1][1]  # never the blocked agent's pane


async def test_m1_an_agent_alerts_pane_turned_blocked_needs_a_deliberate_g(store, fake_herdr, monkeypatch):
    """An agent alert (and a question) whose agent now waits at a prompt: a g typed right
    after another key goes nowhere; a g after a pause does."""
    import asyncio

    monkeypatch.setattr(tui, "ALERT_GO_GRACE", 0.5)
    monkeypatch.setenv("FAKE_HERDR_STATUS", "blocked")
    service.ask(store, A, "Q from the blocked agent?")
    service.ask(store, B, "Other?")
    alerts.raise_alert(store, A, "Check the deploy")
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test() as pilot:
        await asyncio.sleep(0.6)
        await pilot.press("1", "g")  # a question of the blocked pane: refused (live status)
        await pilot.pause()
        await app.workers.wait_for_complete()
        await pilot.press("up", "enter", "g")  # the agent alert: refused (no pause)
        await pilot.pause()
        await app.workers.wait_for_complete()
        assert app.mode == "alert" and not fake_herdr.calls_of("agent", "focus")
        await asyncio.sleep(0.6)
        await pilot.press("g")
        await pilot.pause()
        await app.workers.wait_for_complete()
    assert [c[2] for c in fake_herdr.calls_of("agent", "focus")] == ["w1:p1"]


def test_l1_the_blocked_clock_survives_a_worker_restart(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 15.0)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())
    worker.tick(store)
    store.db.execute("UPDATE blocked_panes SET since = ?", (alerts.now(-__import__("datetime").timedelta(seconds=20)),))
    alerts._unblocked.clear()  # a new worker process
    worker.tick(store)
    assert len(alerts.open_alerts(store)) == 1


async def test_h1_the_only_question_still_opens_directly_with_an_alert_open(store, fake_herdr):
    service.ask(store, B, "Only one?", options=["yes", "no"])
    alerts.raise_alert(store, A, "Check it")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.mode == "group" and app.direct


async def _g_presses(store, fake_herdr, monkeypatch, gaps):
    """An alert view open (reached after a pause), then g presses `gaps` seconds apart."""
    import asyncio

    monkeypatch.setattr(tui, "ALERT_GO_GRACE", 0.5)
    service.ask(store, B, "Q?")
    service.ask(store, B, "Q2?")
    alerts.raise_alert(store, A, "Check the deploy")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("up", "enter")
        await pilot.pause()
        assert app.mode == "alert"
        for gap in gaps:
            await asyncio.sleep(gap)
            await pilot.press("g")
        await pilot.pause()
        await app.workers.wait_for_complete()
    return [c[2] for c in fake_herdr.calls_of("agent", "focus")]


async def test_a_held_g_never_passes_the_pause(store, fake_herdr, monkeypatch):
    # the first g within the pause after Enter, then auto-repeat: 8 g at 80 ms
    assert await _g_presses(store, fake_herdr, monkeypatch, [0.08] * 8) == []


async def test_a_double_tapped_g_never_passes_the_pause(store, fake_herdr, monkeypatch):
    # a g 200 ms after Enter, then a second tap 350 ms later (past the pause from Enter, not
    # from the first g): both refused
    assert await _g_presses(store, fake_herdr, monkeypatch, [0.2, 0.35]) == []


async def test_a_deliberate_g_after_a_pause_still_goes(store, fake_herdr, monkeypatch):
    assert await _g_presses(store, fake_herdr, monkeypatch, [0.1, 0.6]) == ["w1:p1"]
