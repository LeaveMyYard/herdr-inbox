"""The remote-machine app screens, ``tests/fixtures/screens/popup-remote.json`` (P6-D2): run by
``tools/capture_goldens.py`` with its other popup screens, on its held clock and data. A remote
machine's rows are drawn from the local mirror only (``tools/popup_remote.py`` writes it):
its groups, queued, sent, failed and refused actions, an answer it cannot deliver, its alerts,
its question and alert opened, the dismiss confirmation and the placeholders. The Rust renderer
draws them cell for cell (``crates/inbox-core/tests/screens/remote.rs``).
"""

from __future__ import annotations

import asyncio

QUEUED_UNTIL = "2099-01-01T15:30:00+00:00"  # past the real clock the popup compares with: "expires 15:30"
LONG_AGO = "2000-01-01T00:00:00+00:00"


async def remote_screens(shots, work, h) -> None:
    """``h``: tools/capture_goldens.py (its Shots, fresh_store, callers and hold)."""
    import popup_remote as r

    from herdr_inbox import remote, service
    from herdr_inbox.tui import InboxApp

    r.hold()
    r.reset()
    remote.settings = lambda: {"sync_seconds": 5.0, "expiry_hours": 24 * 365 * 100.0}  # never past the real clock
    billing, search, docs = h.callers()
    size = (110, 30)
    src = "InboxApp, a remote machine's rows (the local mirror), focus pane w1:p1"
    file = "popup-remote"

    def mark(store, request_id, **cols):
        sets = ", ".join(f"{k} = ?" for k in cols)
        store.db.execute(f"UPDATE remote_outbox SET {sets} WHERE request_id = ?", (*cols.values(), request_id))

    store = h.fresh_store(work, "remote")
    service.ask(store, billing, "Publish the v2 docs now or after the release?", options=["now", "after"],
                header="Rollout")                                                                       # Q1
    r.machine(store, offline=True)
    r.ralert(store, 1, "Deploy needs your eyes")
    r.ralert(store, 2, "Approval prompt", kind="blocked")
    r.rq(store, 3, "Ship the release today?", "yes", "no", group=3, header="Ship", tab_label="Release",
         asked_by="releaser")
    r.rq(store, 4, "Tag it as v2.0?", "yes", "no", group=3, header="Tag", tab_label="Release", asked_by="releaser")
    for rid, text in ((5, "Rotate the keys?"), (6, "Drop the cache?"), (7, "Bump the quota?"), (8, "Purge logs?")):
        r.rq(store, rid, text, "yes", "no")
    mark(store, r.outbox(store, 5, "answer"), expires_at=QUEUED_UNTIL)
    mark(store, r.outbox(store, 6, "hide"), state="rejected", result="answered with no")
    mark(store, r.outbox(store, 7, "answer"), expires_at=LONG_AGO)
    mark(store, r.outbox(store, 8, "dismiss"), expires_at=QUEUED_UNTIL, sending_at=r._iso(r.HELD))
    r.rq(store, 9, "Restart the worker?", "yes", status="answered", answer="yes", agent_gone=True)
    h.hold(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        app._poll_timer.stop()  # the shots poll by hand: a timed poll lists the new outbox row at random
        await shots.add(file, src, size, "remote rows", app, pilot)
        await pilot.press("down")
        await shots.add(file, src, size, "cursor on a remote group", app, pilot)
        await pilot.press(*["down"] * 6)
        await shots.add(file, src, size, "cursor on an undeliverable remote answer", app, pilot)
        await pilot.press(*["up"] * 6, "enter")
        await shots.add(file, src, size, "remote question opened", app, pilot)
        await pilot.press("down", "n", *"soon")
        await shots.add(file, src, size, "a note on a remote question", app, pilot)
        await pilot.press("escape", "escape", "d")
        await shots.add(file, src, size, "remote dismiss confirmation", app, pilot)
        await pilot.press("n", "up", "up", "up", "enter")
        await shots.add(file, src, size, "remote alert opened", app, pilot)
        await pilot.press("escape", "down", "enter")
        await shots.add(file, src, size, "blocked remote alert opened", app, pilot)
        await pilot.press("escape", "down", "down", "enter", "enter")
        await shots.add(file, src, size, "first of two answered, the row stays", app, pilot)
        app.poll()
        await shots.add(file, src, size, "after a poll", app, pilot)
        await pilot.press("enter")
        store.db.execute("UPDATE remote_questions SET status = 'withdrawn' WHERE remote_id = 4")
        app.poll()
        await shots.add(file, src, size, "remote question closed there", app, pilot)
        await asyncio.sleep(0.6)  # past GRACE_SECONDS: the view changed under the operator
        await pilot.press("escape", "up", "up", "up", "d")
        await shots.add(file, src, size, "remote alert acknowledged in the list", app, pilot)
    store.close()

    store = h.fresh_store(work, "remote-unlabelled")
    service.ask(store, billing, "Publish the v2 docs now or after the release?", options=["now", "after"])
    r.machine(store, mid="m2", label=None)
    r.rq(store, 1, "Ship it?", "yes", mid="m2", title="Release train")
    h.hold(store)
    app = InboxApp(store, focus_pane="w1:p1")
    async with app.run_test(size=size) as pilot:
        app._poll_timer.stop()  # the shots poll by hand: a timed poll lists the new outbox row at random
        await pilot.press("down", "enter")
        await shots.add(file, src, size, "an unlabelled machine's question", app, pilot)
        await pilot.press("enter")
        await shots.add(file, src, size, "an unlabelled machine's row answered", app, pilot)
    store.close()
