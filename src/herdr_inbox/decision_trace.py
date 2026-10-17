"""Test-only trace of delivery decisions: the delivery decision table golden
(docs/plans/rust-rewrite.md §5.2 item 5, ``tests/fixtures/decisions.json``).

``RECORDER`` is None in every real process: nothing in the package sets it, and no
environment variable or file turns it on. Only the test plugin ``tests/decision_recorder.py``
(loaded with ``pytest -p tests.decision_recorder``) sets it. While it is None, ``begin``
returns None, ``track``, ``end`` and ``event`` return at once, and ``note`` hands back its
value untouched: no database read, no write, no herdr call, no change to what the caller
does. Standard library only (the hooks import this).
"""

from __future__ import annotations

RECORDER = None  # tests/decision_recorder.py's Recorder while a capture runs; None otherwise


def begin(path: str, store=None, qs=(), **inputs):
    """A trace frame for one decision (one pane of a deliver_batch, one hook firing, one
    verification), or None when off. ``qs`` (questions or ids) get their state read now."""
    rec = RECORDER
    return None if rec is None else rec.begin(path, store, qs, inputs)


def track(frame, store, qs) -> None:
    """More questions for ``frame`` (their state read now, before any claim)."""
    if frame is not None:
        frame.track(store, qs)


def note(frame, key: str, value, qids=None):
    """Record ``value`` as observation ``key`` (of the pane, or of each id in ``qids``) and
    return it unchanged: wraps an expression without changing its value or evaluation."""
    if frame is not None:
        frame.note(key, value, qids)
    return value


def kept(frame, key: str, items, rest):
    """Record, per question of ``items``, whether a step took it away (``key`` true: it is not in
    ``rest``), and return ``rest`` unchanged."""
    if frame is not None:
        for q in items:
            frame.note(key, q not in rest, [q.id])
    return rest


def end(frame) -> None:
    """The decision is carried out: read every tracked question's state again, emit rows."""
    if frame is not None:
        frame.end()


def event(path: str, **fields) -> None:
    """A decision with no question state of its own (the rewake watcher's poll)."""
    rec = RECORDER
    if rec is not None:
        rec.event(path, fields)
