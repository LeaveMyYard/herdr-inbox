"""Review of #2: L3 batch splitting, L4 never-attempted retry, L5 unambiguous picks, row alignment."""

from rich.text import Text
from textual.content import Content

from herdr_inbox import herdr, service
from herdr_inbox.service import Caller
from herdr_inbox.store import Question
from herdr_inbox.tui import QuestionView

A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")


def test_long_batch_is_split_not_truncated(store, fake_herdr):
    ids = [service.ask(store, A, f"Question {i}?")["id"] for i in range(3)]
    for qid in ids:
        store.set_draft(qid, service.make_detail("x" * 1800))
    service.submit(store, ids)
    prompts = [c[3] for c in fake_herdr.calls_of("agent", "prompt")]
    assert len(prompts) == 2 and all(len(p) <= herdr.MAX_ARG for p in prompts)
    assert not any(p.endswith("…") for p in prompts)
    joined = " ".join(prompts)
    assert all(f"Q{i}" in joined for i in ids) and joined.count("x" * 1800) == 3
    assert all(store.get(i).delivered_at and store.get(i).acknowledged_at for i in ids)


def test_answer_longer_than_a_message_is_referenced(store, fake_herdr):
    qid = service.ask(store, A, "Essay?")["id"]
    q = service.answer(store, qid, "y" * 5000)
    prompt = fake_herdr.calls_of("agent", "prompt")[0][3]
    assert "y" * 100 not in prompt and "check_answers" in prompt and "5000" in prompt
    assert q.delivered_at and q.acknowledged_at is None  # the agent still has to fetch it
    assert [a.answer for a in store.unacknowledged_answers("sess-A", "w1:p1")] == ["y" * 5000]


def test_retry_picks_up_answers_never_attempted(store, fake_herdr):
    qid = service.ask(store, A, "Crash?")["id"]
    store.set_answer(qid, "yes")  # answered, then the process died before delivering
    service.retry_deliveries(store, "w1:p1")
    assert store.get(qid).delivered_at is None  # within the grace period: maybe still in flight
    store.db.execute("UPDATE questions SET answered_at = '2020-01-01T00:00:00+00:00' WHERE id = ?", (qid,))
    service.retry_deliveries(store, "w1:p1")
    assert store.get(qid).delivered_at


def test_picks_with_separators_are_quoted():
    assert service.format_answer(["Postgres", "SQLite"]) == "Postgres, SQLite"
    assert service.format_answer(["a, b", "c"], {"c": "n"}) == '"a, b", c (note: n)'
    assert service.format_answer(['say "hi"', "x (y)"]) == '"say \\"hi\\"", "x (y)"'


def test_option_rows_share_one_cursor_column(store, fake_herdr):
    qid = service.ask(store, A, "Which?", options=[{"label": "Postgres", "description": "d"}, "SQLite"],
                      multi_select=False)["id"]
    v = QuestionView(store.get(qid))
    v.cursor = 1
    firsts = [Content.from_markup(v.row_text(i)).plain.split("\n")[0] for i in range(3)]
    starts = {line.index(label) for line, label in zip(firsts, ["Postgres", "SQLite", "Type"])}
    assert len(starts) == 1, firsts  # labels start in the same cell on every row
    assert firsts[1].startswith("❯ ") and firsts[0].startswith("  ")
