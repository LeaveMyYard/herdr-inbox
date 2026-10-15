from herdr_inbox import cli, service
from herdr_inbox.service import Caller


def ask(store, question="Merge PR 42 now?", pane="w1:p1"):
    c = Caller(pane_id=pane, tab_id="w1:t1", tab_label="my-tab", session_id="sess-A", cwd="/repo")
    return service.ask(store, c, question)["id"]


def test_answer_is_prompted_into_the_asking_pane(store, fake_herdr):
    qid = ask(store)
    fake_herdr.reset()
    q = service.answer(store, qid, "yes, squash it")
    assert q.status == "answered" and q.delivered_at and q.delivery_error is None
    assert fake_herdr.calls() == [
        ["agent", "get", "w1:p1"],
        ["pane", "read", "w1:p1", "--source", "visible", "--lines", "20"],  # 0.6.1: panel focus check
        ["agent", "prompt", "w1:p1", "[inbox] Q1 answered — Merge PR 42 now?: yes, squash it"],
        ["pane", "report-metadata", "w1:p1", "--source", "herdr-inbox", "--clear-token", "inbox_q"],
    ]
    # never --wait: delivery must not block the TUI on the agent's turn
    assert all("--wait" not in c for c in fake_herdr.calls())


def test_long_question_is_truncated_in_delivery(store, fake_herdr):
    qid = ask(store, "x" * 200)
    service.answer(store, qid, "ok")
    prompt = fake_herdr.calls_of("agent", "prompt")[0][3]
    assert prompt == f"[inbox] Q1 answered — {'x' * 79}…: ok"


def test_failed_prompt_records_delivery_error(store, fake_herdr, monkeypatch):
    qid = ask(store)
    monkeypatch.setenv("FAKE_HERDR_FAIL", "agent prompt")
    q = service.answer(store, qid, "yes")
    assert q.status == "answered" and q.delivered_at is None
    assert "fake failure" in q.delivery_error
    # still retrievable by the agent
    assert [a.id for a in store.unacknowledged_answers("sess-A", "w1:p1")] == [qid]


def test_no_live_agent_skips_prompt(store, fake_herdr, monkeypatch):
    qid = ask(store)
    monkeypatch.setenv("FAKE_HERDR_AGENT", "0")
    fake_herdr.reset()
    q = service.answer(store, qid, "yes")
    assert q.delivery_error == "no live agent in pane w1:p1"
    assert fake_herdr.calls_of("agent", "prompt") == []


def test_missing_herdr_binary_degrades(store, fake_herdr, monkeypatch):
    qid = ask(store)
    monkeypatch.setenv("HERDR_BIN_PATH", "/nonexistent/herdr")
    q = service.answer(store, qid, "yes")
    assert q.status == "answered" and q.delivery_error


def test_answering_twice_is_refused(store, fake_herdr):
    qid = ask(store)
    assert service.answer(store, qid, "a") is not None
    assert service.answer(store, qid, "b") is None
    assert store.get(qid).answer == "a"


def test_token_counts_follow_state(store, fake_herdr):
    a = ask(store, "one?")
    ask(store, "two?")
    tokens = [c[-1] for c in fake_herdr.calls_of("pane", "report-metadata")]
    assert tokens == ["inbox_q=?1", "inbox_q=?2"]
    fake_herdr.reset()
    service.close(store, a, "dismissed")
    assert fake_herdr.calls() == [["pane", "report-metadata", "w1:p1", "--source", "herdr-inbox", "--token", "inbox_q=?1"]]


def test_sync_tokens_sets_and_clears_every_pane(store, fake_herdr):
    ask(store, "one?")
    ask(store, "two?")
    fake_herdr.reset()
    service.sync_tokens(store)
    calls = fake_herdr.calls()
    assert calls[0] == ["pane", "list"]
    assert fake_herdr.calls_of("pane", "report-metadata")[:2] == [
        ["pane", "report-metadata", "w1:p1", "--source", "herdr-inbox", "--token", "inbox_q=?2"],
        ["pane", "report-metadata", "w1:p2", "--source", "herdr-inbox", "--clear-token", "inbox_q"],
    ]


def test_sync_tokens_cli(store, fake_herdr):
    ask(store, "one?", pane="w1:p2")
    fake_herdr.reset()
    assert cli.main(["sync-tokens"]) == 0
    assert ["pane", "report-metadata", "w1:p2", "--source", "herdr-inbox", "--token", "inbox_q=?1"] in fake_herdr.calls()


def test_cli_answer_list_dismiss(store, fake_herdr, capsys):
    a = ask(store, "one?")
    b = ask(store, "two?")
    assert cli.main(["list"]) == 0
    out = capsys.readouterr().out
    assert "Q1" in out and "Q2" in out and "my-tab" in out
    assert cli.main(["answer", f"Q{a}", "go", "ahead"]) == 0
    assert "Q1 answered, delivered to w1:p1" in capsys.readouterr().out
    assert fake_herdr.calls_of("agent", "prompt")[-1][-1].endswith(": go ahead")
    assert cli.main(["dismiss", str(b)]) == 0
    assert store.get(b).status == "dismissed"
    assert cli.main(["dismiss", str(b)]) == 1


ESCAPES = "\x1b[201~\x1b[Z\x07\x9b2J\x7f"


def test_control_characters_never_reach_herdr(store, fake_herdr):
    """H3: agent-written text must not smuggle terminal escapes into prompts/notifications."""
    qid = ask(store, f"merge?{ESCAPES}then\tok x")
    service.answer(store, qid, f"yes{ESCAPES}\nsecond line")
    sent = [arg for call in fake_herdr.calls() for arg in call]
    for bad in ("\x1b", "\x07", "\x9b", "\x7f", "\t", "\n", " "):
        assert not any(bad in arg for arg in sent), repr(bad)
    prompt = fake_herdr.calls_of("agent", "prompt")[0][3]
    assert prompt == "[inbox] Q1 answered — merge?[201~[Z2Jthen ok: yes[201~[Z2J second line"
    title, body = fake_herdr.calls_of("notification", "show")[0][2:5:2]
    assert "\x1b" not in title + body and body.startswith("merge?[201~")
