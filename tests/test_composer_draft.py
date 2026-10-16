"""Live bug 2026-10-16 (Q124/Q125): a batch was typed onto the operator's unsent draft in
Claude Code's input box and submitted with it; the anchored matcher then missed it and Send
now typed it again. Fixes: never type into a Claude input box that holds text; a delivery
found verbatim inside a user record (after other text) counts as landed."""

import json
from pathlib import Path

import pytest

from herdr_inbox import herdr, service, store as st, transcript, worker
from herdr_inbox.service import Caller

FIX = Path(__file__).parent / "fixtures" / "composer"
A = Caller("w1:p1", "w1:t1", "tab-A", "sess-A", "/r")


def screen(name):
    return (FIX / f"{name}.ansi").read_text()


@pytest.mark.parametrize("name,state", [("empty", "empty"), ("draft", "draft"), ("wrapped", "draft"),
                                        ("labelled_empty", "empty"), ("labelled_draft", "draft"),
                                        # real captures (#37 review): focused idle box, pasted-text and image chips
                                        ("focused_empty", "empty"), ("pasted_text", "draft"), ("image_chip", "draft")])
def test_the_input_box_state_is_read_from_the_captured_screens(fake_herdr, monkeypatch, name, state):
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", screen(name))
    assert herdr.composer_state("w1:p1") == state


def variant(prompt_line):
    """The captured empty screen with its ❯ line replaced (shapes the review asked about)."""
    lines = screen("empty").split("\n")
    i = next(k for k, line in enumerate(lines) if "\u276f" in line)
    lines[i] = prompt_line
    return "\n".join(lines)


@pytest.mark.parametrize("line,state", [
    ("\u276f\u00a0\x1b[0m\x1b[2m[Pasted text #1 +3 lines]\x1b[0m\r", "draft"),  # a DIM chip is still text
    ("\u276f\u00a0\x1b[0m\x1b[2m[Image #1]\x1b[0m\r", "draft"),
    ("\u276f\u00a0\x1b[7mT\x1b[0m\x1b[2mry \"refactor <filepath>\"\x1b[0m\r", "empty"),  # cursor cell on the placeholder
    ("\u276f\u00a0\x1b[0m\x1b[2mTry \"x\"\x1b[0m and more\r", "draft"),  # text after the placeholder
    ("\u276f\u00a0Try \"x\"\r", "draft"),  # the placeholder's words, typed (not dim)
    ("\u276f\u00a0\r", "empty"),
])
def test_empty_is_recognised_positively(fake_herdr, monkeypatch, line, state):
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", variant(line))
    assert herdr.composer_state("w1:p1") == state


@pytest.mark.parametrize("text", ["", "plain output with no input box\n", "\x1b[2m────\x1b[0m\nno prompt\n\x1b[2m────\x1b[0m"])
def test_no_recognisable_input_box_is_unknown(fake_herdr, monkeypatch, text):
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", text)
    assert herdr.composer_state("w1:p1") == "unknown"


def answered(store, monkeypatch, q="Merge it?", a="squash", screen_name="draft", force=False):
    qid = service.ask(store, A, q)["id"]
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", screen(screen_name) if screen_name else "")
    service.answer(store, qid, a)
    return qid


@pytest.mark.parametrize("name", ["draft", "wrapped", None])
def test_never_typed_onto_a_draft_or_an_unreadable_claude_input_box(store, fake_herdr, monkeypatch, name):
    qid = answered(store, monkeypatch, screen_name=name)
    assert not fake_herdr.calls_of("agent", "prompt")
    assert "input box" in store.get(qid).delivery_error


def test_send_now_never_types_onto_a_draft_and_says_so(store, fake_herdr, monkeypatch):
    qid = answered(store, monkeypatch)
    fake_herdr.reset()
    service.deliver(store, store.get(qid), retry=True, force=True)
    assert not fake_herdr.calls_of("agent", "prompt")
    assert any("input box" in c[4] for c in fake_herdr.calls_of("notification", "show"))


def test_typed_once_the_draft_is_gone(store, fake_herdr, monkeypatch):
    qid = answered(store, monkeypatch)
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", screen("empty"))
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    worker.tick(store)
    assert fake_herdr.calls_of("agent", "prompt")


def test_other_agents_are_typed_into_as_before(store, fake_herdr, monkeypatch):
    monkeypatch.setenv("FAKE_HERDR_AGENT_KIND", "codex")
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    answered(store, monkeypatch, screen_name=None)
    assert fake_herdr.calls_of("agent", "prompt")


# The incident's record SHAPE (captured 2026-10-16, transcript line 4713): the operator's draft
# glued straight onto our batch, one `user` record, origin human. Texts neutralised: the repo
# is public.
DRAFT = "Ok, so how is it going? Is it done yet?"


def merged_record(ts, batch):
    return json.dumps({"type": "user", "timestamp": ts, "origin": {"kind": "human"},
                       "message": {"role": "user", "content": DRAFT + batch}}) + "\n"


def two_answers(store):
    q1 = service.ask(store, A, "ghostpaw-e2e: the new check (branch feat/x) fails any landing referrer; keep 40/20/0 weights or change them?")["id"]
    q2 = service.ask(store, A, "Should I open a terraform PR for the collector origin?")["id"]
    for qid, ans in ((q1, "Keep 40/20/0"), (q2, '"Yes, open TF PR"')):
        store.set_draft(qid, service.make_detail(ans))
    service.submit(store, [q1, q2], deliver=False)
    return [store.get(q1), store.get(q2)]


def test_a_delivery_typed_after_a_draft_counts_as_landed(store, fake_herdr, tmp_path):
    qs = two_answers(store)
    batch = herdr.clean(service.delivery_text(qs), limit=10**9)
    path = tmp_path / "t.jsonl"
    path.write_text(merged_record(st.now(), batch))
    expected = {q.id: (service.expected_segments(q), q.answered_at) for q in qs}
    assert set(transcript.landed_kinds(path, expected)) == {q.id for q in qs}
    assert transcript.landed_since(path, 0, [q.id for q in qs], text=batch)


def test_a_mere_mention_after_other_text_still_does_not_count(store, fake_herdr, tmp_path):
    qs = two_answers(store)
    path = tmp_path / "t.jsonl"
    path.write_text(merged_record(st.now(), f" (about [inbox] answered — Q{qs[0].id} earlier)"))
    expected = {q.id: (service.expected_segments(q), q.answered_at) for q in qs}
    assert transcript.landed_kinds(path, expected) == {}
    assert not transcript.landed_since(path, 0, [qs[0].id], text=service.delivery_text(qs))


def test_send_now_after_a_merged_landing_records_it_instead_of_typing_again(store, fake_herdr, monkeypatch, tmp_path):
    """The incident's second half: Send now ran the pre-check, but the anchored matcher
    missed the merged record, so it typed Q124/Q125 again."""
    qs = two_answers(store)
    path = tmp_path / "sess-A.jsonl"
    path.write_text(merged_record(st.now(), herdr.clean(service.delivery_text(qs), limit=10**9)))
    store.record_session("sess-A", str(path))
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", screen("empty"))
    service.deliver_batch(store, [store.get(q.id) for q in qs], retry=True, force=True)
    assert not fake_herdr.calls_of("agent", "prompt")
    assert all(store.get(q.id).acknowledged_at for q in qs)


def test_l1_a_draft_with_a_blank_first_line_is_a_draft(fake_herdr, monkeypatch):
    """#37 L1 pin: Shift+Enter first, text on the next box line (the surviving mutant that
    ignores lines after the ❯ line)."""
    lines = screen("empty").split("\n")
    i = next(k for k, line in enumerate(lines) if "❯" in line)
    lines[i:i + 1] = ["❯ \r", "  second line of a draft\r"]
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", "\n".join(lines))
    assert herdr.composer_state("w1:p1") == "draft"


def test_l2_an_unreadable_input_box_says_why_once(store, fake_herdr, monkeypatch):
    monkeypatch.setattr(service, "UNREADABLE_NOTIFY_AFTER", 0.0)
    service._unreadable_since.clear()
    service._unreadable_notified.clear()
    qid = answered(store, monkeypatch, screen_name=None)  # unknown: not typed
    for _ in range(3):
        worker.tick(store)
    notes = [c for c in fake_herdr.calls_of("notification", "show") if "cannot read the agent's input box" in c[2]]
    assert len(notes) == 1 and f"Q{qid}" in notes[0][4]
    assert not fake_herdr.calls_of("agent", "prompt")
    monkeypatch.setenv("FAKE_HERDR_SCREEN_ANSI", screen("empty"))  # readable again: re-armed
    monkeypatch.setattr(service, "VERIFY_SECONDS", 0.0)
    worker.tick(store)
    assert "w1:p1" not in service._unreadable_notified


def test_l2_a_brief_unreadable_read_does_not_notify(store, fake_herdr, monkeypatch):
    service._unreadable_since.clear()
    service._unreadable_notified.clear()
    answered(store, monkeypatch, screen_name=None)
    worker.tick(store)
    assert not [c for c in fake_herdr.calls_of("notification", "show") if "cannot read" in c[2]]
