"""A question or alert from a herdr-core terminal, as an old Python process meets it.

Python stays at schema revision 1 while Rust moves on (herdr-app-inbox plan §3.2, "Old Python
processes"). The popup is Python until P6-E, and a rollback to `engine` python puts the worker
back on this code. So these tests pin what this Python does to a database at Rust's revision
holding core rows: it opens it, shows the rows, types nothing, and writes no outcome. The rows
are the shape Rust files (`src/mcp/ask/file.rs`): `pane_id` NULL, `herdr_server` 'herdr-core', a
`question_host` row.
"""

import json
from datetime import timedelta
from pathlib import Path

import pytest

from herdr_inbox import alerts, service, store as st, worker
from herdr_inbox.service import Caller
from herdr_inbox.store import Store
from herdr_inbox.tui import GroupListView, InboxApp

OWNED = json.loads((Path(__file__).parent / "fixtures" / "migrate-rust.json").read_text())
SESSION = "aaaaaaaa-0000-4000-8000-00000000000a"
CORE = "herdr-core"
A_PANE = Caller("w1:p1", "w1:t1", "tab-A", None, "/r")
AN_HOUR_AGO = timedelta(hours=-1)
TABLES = ("questions", "question_ext", "question_host", "alerts", "alert_host")


def at_rusts_revision(store: Store) -> None:
    """The database as a Rust worker leaves it: its additions, its stamp."""
    for sql in OWNED["statements"]:
        store.db.execute(sql)
    for table, columns in OWNED["added_columns"]:
        for name, decl in columns:
            store.db.execute(f"ALTER TABLE {table} ADD COLUMN {name} {decl}")
    store.db.execute(f"PRAGMA user_version = {st.REVISION_BASE + OWNED['revision']}")


@pytest.fixture
def db(store):
    at_rusts_revision(store)
    return store


def core_question(store: Store, text: str = "Question Title 123") -> int:
    q = store.insert(text, session_id=SESSION, cwd="/r", asked_by="Project Alpha lane")
    store.db.execute("UPDATE question_ext SET herdr_server = ? WHERE id = ?", (CORE, q.id))
    store.db.execute("INSERT INTO question_host (id, host, terminal) VALUES (?, ?, 't3')", (q.id, CORE))
    return q.id


def core_alert(store: Store) -> int:
    cur = store.db.execute(
        "INSERT INTO alerts (created_at, kind, severity, title, session_id, herdr_server)"
        " VALUES (?, 'agent', 'warning', 'Please check Project Alpha', ?, ?)", (st.now(), SESSION, CORE))
    store.db.execute("INSERT INTO alert_host (id, host, terminal) VALUES (?, ?, 't3')", (cur.lastrowid, CORE))
    return cur.lastrowid


def answered(store: Store, qid: int) -> None:
    """The operator answered it in the popup, which records and hands delivery on."""
    store.set_draft(qid, service.make_detail("Yes"))
    service.submit(store, [qid], deliver=False)


def rows(store: Store) -> dict[str, list[tuple]]:
    return {t: [tuple(r) for r in store.db.execute(f"SELECT * FROM {t} ORDER BY id").fetchall()] for t in TABLES}


def typed(fake_herdr) -> list[list[str]]:
    return fake_herdr.calls_of("agent", "prompt")


def stamp(store: Store) -> int:
    return store.db.execute("PRAGMA user_version").fetchone()[0]


def test_python_opens_rusts_revision_and_leaves_it(db, fake_herdr):
    before = stamp(db)
    assert st.stamped_revision(before) > st.MIGRATION_REVISION
    core_question(db)
    with Store(migrate=True) as again:
        assert stamp(again) == before  # a newer stamp is never lowered
        assert again.db.execute("SELECT COUNT(*) FROM question_host").fetchone()[0] == 1
    with Store() as plain:  # a hook's or an MCP server's open: a higher stamp is used as it is
        assert plain.open_counts() == {}


def test_the_worker_never_touches_an_open_core_question(db, fake_herdr):
    core_question(db)
    core_alert(db)
    before = rows(db)
    fake_herdr.reset()
    for _ in range(service.MAX_STALE_RETRIES + 3):
        assert worker.tick(db) == 0  # and nothing of it keeps the worker alive
    assert rows(db) == before
    assert not typed(fake_herdr)
    assert not fake_herdr.calls_of("notification", "show")


def test_the_worker_never_types_or_settles_an_answered_core_question(db, fake_herdr, monkeypatch):
    qid = core_question(db)
    answered(db, qid)
    # past every grace that could make it a candidate
    db.db.execute("UPDATE questions SET answered_at = ? WHERE id = ?", (st.now(AN_HOUR_AGO), qid))
    before = rows(db)
    fake_herdr.reset()
    for _ in range(service.MAX_STALE_RETRIES + 3):
        assert worker.tick(db) == 0
    assert rows(db) == before  # no error, no attempt, no claim, not gone
    assert not typed(fake_herdr)
    assert not fake_herdr.calls_of("notification", "show")
    q = db.get(qid)
    assert (q.delivered_at, q.delivery_error, q.delivery_attempts) == (None, None, 0)


def test_a_core_answer_handed_to_delivery_is_left_as_it_is(db, fake_herdr, monkeypatch):
    """The popup's sender, or `inbox deliver`, names the id: it is another server's row, so
    nothing here claims it. Only that "server's" worker is asked for, which finds no server."""
    qid = core_question(db)
    answered(db, qid)
    asked = []
    monkeypatch.setattr(service, "_ensure_worker", lambda server=None: asked.append(server))
    before = rows(db)
    fake_herdr.reset()
    service.deliver_batch(db, [db.get(qid)], force=True)  # the popup's Retry forces
    assert rows(db) == before
    assert not typed(fake_herdr)
    assert asked == [None, CORE]


def test_the_operator_still_sees_a_core_answer_and_the_agent_can_fetch_it(db, fake_herdr):
    qid = core_question(db)
    answered(db, qid)
    db.db.execute("UPDATE questions SET answered_at = ? WHERE id = ?", (st.now(AN_HOUR_AGO), qid))
    assert [q.id for q in db.failed_deliveries()] == [qid]  # an unattempted ⚠ row
    got = db.unacknowledged_answers(SESSION, None)
    assert [(q.id, q.answer) for q in got] == [(qid, "Yes")]


def test_python_refuses_a_core_caller_as_before(db, fake_herdr, monkeypatch):
    """After a rollback a Python MCP server in a core terminal has no pane: `unavailable`."""
    monkeypatch.delenv("HERDR_PANE_ID")
    monkeypatch.setenv("HC_TERMINAL_ID", "t3")
    monkeypatch.setenv("CLAUDE_CODE_SESSION_ID", SESSION)
    caller = service.resolve_caller()
    assert caller.pane_id is None
    assert service.ask(db, caller, "Question Title 123")["status"] == "unavailable"
    assert alerts.raise_alert(db, caller, "Title")["status"] == "unavailable"
    assert db.db.execute("SELECT COUNT(*) FROM questions").fetchone()[0] == 0


def test_a_pane_question_beside_core_rows_is_delivered_as_ever(db, fake_herdr):
    core = core_question(db)
    answered(db, core)
    qid = service.ask(db, A_PANE, "Other question?")["id"]
    fake_herdr.reset()
    service.answer(db, qid, "Yes")
    assert len(typed(fake_herdr)) >= 1
    assert db.get(qid).delivered_at is not None
    assert db.get(core).delivered_at is None


async def test_the_popup_lists_core_rows_and_acknowledges_a_core_alert(db, fake_herdr):
    core_question(db)
    aid = core_alert(db)
    service.ask(db, A_PANE, "Other question?")
    app = InboxApp(db, focus_pane="w1:p2")
    async with app.run_test() as pilot:
        await pilot.pause()
        body = app.query_one(GroupListView).body_text()
        assert f"A{aid}" in body and "Question Title 123" in body and "no pane" in body
    assert alerts.acknowledge(db, aid) is not None
    assert alerts.get(db, aid).status == "acked"
