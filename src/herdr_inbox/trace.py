"""Step timings for latency investigations: ``INBOX_TRACE=<file>`` (off by default).

Each step appends one JSON line ``{"t": <unix>, "pid", "thread", "step", "ms"}`` to the file,
from any process (popup, MCP server, worker, CLI). Off, ``step`` costs one env lookup.
"""

from __future__ import annotations

import contextlib
import json
import os
import threading
import time


def enabled() -> bool:
    return bool(os.environ.get("INBOX_TRACE"))


def record(step: str, ms: float, **extra) -> None:
    path = os.environ.get("INBOX_TRACE")
    if not path:
        return
    line = {"t": round(time.time(), 4), "pid": os.getpid(), "thread": threading.current_thread().name,
            "step": step, "ms": round(ms, 2), **extra}
    try:
        with open(path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(line) + "\n")
    except OSError:
        pass


@contextlib.contextmanager
def step(name: str, **extra):
    if not enabled():
        yield
        return
    start = time.perf_counter()
    try:
        yield
    finally:
        record(name, (time.perf_counter() - start) * 1000, **extra)


def traced(name: str):
    """Decorator: time every call of a (sync) function as step ``name``."""
    import functools

    def wrap(fn):
        @functools.wraps(fn)
        def inner(*args, **kwargs):
            with step(name):
                return fn(*args, **kwargs)
        return inner
    return wrap
