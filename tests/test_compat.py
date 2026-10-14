"""H1 (review of #2): 0.1.x processes keep working after 0.2 has opened and written the DB."""

import dataclasses
import sqlite3

from herdr_inbox import service, store as store_v02
from herdr_inbox.service import Caller

from . import compat_v01_store as v01

A = Caller("w1:p1", "w1:t1", "tab", "sess-A", "/r")


def test_v01_reads_and_writes_after_v02_migrated_and_wrote(tmp_path, fake_herdr):
    path = tmp_path / "inbox.db"
    old = v01.Store(path)
    legacy = old.insert("Old question?", options=["yes", "no"], pane_id="w1:p1", session_id="sess-A")
    new = store_v02.Store(path)  # 0.2 opens (and migrates) the same DB
    group = service.ask_group(new, A, [{"question": "DB?", "header": "DB", "options": [{"label": "pg", "description": "x"}],
                                        "multi_select": True}, {"question": "Cache?"}])
    ids = [q["id"] for q in group["questions"]]
    new.set_draft(ids[1], service.make_detail("redis"))
    service.answer(new, ids[0], choices=["pg"], notes={"pg": "keep schema"})

    # the 0.1.x process, still running, reads every row and keeps writing
    rows = old.list(include_closed=True)
    assert sorted(q.id for q in rows) == [legacy.id, *ids]
    assert old.get(ids[0]).answer == "pg (note: keep schema)"
    assert old.get(ids[0]).options == ["pg"]  # N3: plain labels, as a 0.1.x popup escape()s them
    assert new.get(ids[0]).options == [{"label": "pg", "description": "x", "preview": None}]
    assert [q.id for q in old.mine("sess-A", "w1:p1")] == [legacy.id, ids[0], ids[1]]
    assert old.find_duplicate(v01.normalize("cache"), "sess-A", "w1:p1").id == ids[1]
    assert old.set_answer(ids[1], "memcached")
    assert [q.id for q in old.unacknowledged_answers("sess-A", "w1:p1")] == [ids[1]]

    # and 0.2 still sees what 0.1.x wrote
    assert new.get(ids[1]).answer == "memcached" and new.get(legacy.id).group == legacy.id


def test_questions_table_keeps_the_v01_column_set(tmp_path, fake_herdr):
    path = tmp_path / "inbox.db"
    v01.Store(path).close()
    before = [r[1] for r in sqlite3.connect(path).execute("PRAGMA table_info(questions)")]
    store_v02.Store(path).close()
    after = [r[1] for r in sqlite3.connect(path).execute("PRAGMA table_info(questions)")]
    assert after == before


def test_from_row_ignores_columns_it_does_not_know(tmp_path, fake_herdr):
    s = store_v02.Store(tmp_path / "inbox.db")
    s.db.execute("ALTER TABLE questions ADD COLUMN added_in_v03 TEXT")  # a future version
    qid = service.ask(s, A, "Still readable?")["id"]
    assert s.get(qid).question == "Still readable?"
    assert "added_in_v03" not in {f.name for f in dataclasses.fields(store_v02.Question)}


def test_views_are_replaced_not_kept(tmp_path, fake_herdr):
    """N4: an older view definition is dropped and the versioned one created."""
    path = tmp_path / "inbox.db"
    db = sqlite3.connect(path)
    db.executescript(store_v02.SCHEMA)
    db.execute("CREATE VIEW questions_v2 AS SELECT id FROM questions")  # a pre-release definition
    db.commit()
    db.close()
    store_v02.Store(path).close()
    views = {r[0] for r in sqlite3.connect(path).execute("SELECT name FROM sqlite_master WHERE type = 'view'")}
    assert views == {store_v02.VIEW}
