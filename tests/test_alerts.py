"""Alerts (0.8): an agent's "please check X", and agents stuck at a blocking prompt."""

import json

from herdr_inbox import alerts, service, worker
from herdr_inbox.service import Caller
from herdr_inbox.tui import AlertView, GroupListView, InboxApp

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


async def test_popup_shows_alerts_first_and_enter_then_1_acknowledges(store, fake_herdr):
    service.ask(store, B, "A question?")
    service.ask(store, B, "Another?")
    aid = alerts.raise_alert(store, A, "Disk almost full", "92% on /data", "urgent")["id"]
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert body.startswith(f"❯ 1. ⚑ A{aid} · urgent · Disk almost full")
        await pilot.press("enter")
        await pilot.pause()
        assert app.mode == "alert" and "92% on /data" in str(app.query_one(AlertView).query(".question")[1].render())
        await pilot.press("1")
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
        await pilot.press("d")
        await pilot.pause()
    assert alerts.get(store, a1).status == "acked"
    alerts.raise_alert(store, A, "Two")
    app = InboxApp(store, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        await pilot.press("g")
        await app.workers.wait_for_complete()
        await pilot.pause()
    assert fake_herdr.calls_of("agent", "focus") or fake_herdr.calls_of("pane", "focus")


def blocked(pane="w1:p1", status="blocked"):
    return json.dumps([{"pane_id": pane, "tab_id": "w1:t1", "agent": "claude", "agent_status": status,
                        "agent_session": {"value": "sess-A"}}])


def test_an_agent_stuck_at_a_prompt_becomes_an_alert_and_clears_itself(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 0.0)
    alerts._blocked_since.clear()
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())
    worker.tick(store)
    worker.tick(store)
    (a,) = alerts.open_alerts(store)
    assert a.kind == "blocked" and a.title == "my-tab is waiting for you"
    assert len([c for c in fake_herdr.calls_of("notification", "show") if "waiting" in c[4]]) == 1
    assert store.open_counts() == {"w1:p1": 1}
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked(status="working"))
    worker.tick(store)
    assert alerts.open_alerts(store) == [] and alerts.get(store, a.id).status == "cleared"
    assert not fake_herdr.calls_of("agent", "prompt") and not fake_herdr.calls_of("pane", "send-keys")


def test_a_short_prompt_never_raises_an_alert(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(alerts, "BLOCKED_AFTER", 3600.0)
    alerts._blocked_since.clear()
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
        await pilot.press("enter")
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
    alerts._blocked_since.clear()
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())
    worker.tick(store)
    (a,) = alerts.open_alerts(store)
    alerts.acknowledge(store, a.id)  # the operator dismissed it; the prompt is still up
    worker.tick(store)
    worker.tick(store)
    assert alerts.open_alerts(store) == []
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked(status="working"))
    worker.tick(store)
    monkeypatch.setenv("FAKE_HERDR_AGENTS", blocked())  # the next prompt alerts again
    worker.tick(store)
    assert len(alerts.open_alerts(store)) == 1
