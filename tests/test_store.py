from herdr_inbox import service
from herdr_inbox.service import Caller
from herdr_inbox.store import normalize


def caller(session="sess-A", pane="w1:p1"):
    return Caller(pane_id=pane, tab_id="w1:t1", tab_label="my-tab", session_id=session, cwd="/repo")


def test_normalize_ignores_case_punctuation_and_whitespace():
    assert normalize("  Should I MERGE   PR #595?\n") == normalize("should i merge pr 595")


def test_new_question_is_recorded(store, fake_herdr):
    out = service.ask(store, caller(), "Merge PR 595?", options=["yes", "no"], asked_by="reviewer")
    assert out["status"] == "open"
    assert "Recorded as Q1" in out["message"] and "`[inbox]` that names Q1" in out["message"]
    q = store.get(out["id"])
    assert (q.session_id, q.pane_id, q.tab_label, q.options, q.asked_by) == (
        "sess-A", "w1:p1", "my-tab", [{"label": "yes", "description": None}, {"label": "no", "description": None}],
        "reviewer",
    )


def test_open_duplicate_is_not_reinserted(store, fake_herdr):
    first = service.ask(store, caller(), "Merge PR 595?")
    again = service.ask(store, caller(), "merge   pr 595")
    assert again == {
        "id": first["id"],
        "status": "already_open",
        "message": f"Q{first['id']} is already with the operator. Do not ask again; keep working.",
    }
    assert store.get(first["id"]).repeat_count == 1
    assert len(store.list()) == 1


def test_answered_duplicate_returns_the_answer(store, fake_herdr):
    first = service.ask(store, caller(), "Which region?")
    service.answer(store, first["id"], "eu-west-1")
    again = service.ask(store, caller(), "which region")
    assert again["status"] == "answered" and again["answer"] == "eu-west-1"
    assert again["id"] == first["id"]
    assert len(store.list(include_closed=True)) == 1


def test_different_session_is_not_deduped(store, fake_herdr):
    a = service.ask(store, caller(session="sess-A"), "Which region?")
    b = service.ask(store, caller(session="sess-B"), "Which region?")
    assert b["status"] == "open" and b["id"] != a["id"]


def test_dismissed_question_can_be_asked_again(store, fake_herdr):
    a = service.ask(store, caller(), "Which region?")
    assert service.close(store, a["id"], "dismissed")
    b = service.ask(store, caller(), "Which region?")
    assert b["status"] == "open" and b["id"] != a["id"]


def test_pane_is_the_fallback_owner_without_session(store, fake_herdr):
    a = service.ask(store, caller(session=None), "Which region?")
    same_pane = service.ask(store, caller(session=None), "Which region?")
    other_pane = service.ask(store, caller(session=None, pane="w1:p2"), "Which region?")
    assert same_pane["status"] == "already_open" and same_pane["id"] == a["id"]
    assert other_pane["status"] == "open"


def test_ask_notifies_and_sets_token(store, fake_herdr):
    service.ask(store, caller(), "Merge PR 595?\nsecond line")
    assert fake_herdr.calls_of("notification", "show") == [
        ["notification", "show", "Q1 · my-tab", "--body", "Merge PR 595?", "--sound", "request"]
    ]
    assert fake_herdr.calls_of("pane", "report-metadata") == [
        ["pane", "report-metadata", "w1:p1", "--source", "herdr-inbox", "--token", "inbox_q=?1"]
    ]
