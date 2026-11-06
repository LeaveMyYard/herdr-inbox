"""The feed runtime's files under data/herdr-inbox (docs/protocol/app-feed-v1.md, Runtime).

Nothing here signals a process. An uninstall disables, then waits for the writer to leave."""
import fcntl
import json
import os
import time
from contextlib import contextmanager

from receipt import DATA

CONTROL = DATA + '/control.json'
ENSURE_LOCK = DATA + '/ensure.lock'
WRITER_LOCK = DATA + '/writer.lock'
START = DATA + '/start.json'
FILES = ('feed.json', '.feed.tmp', 'writer.json', 'stop.request', 'start.json', 'control.json',
         'writer.lock', 'ensure.lock')
# `--ensure` holds its lock for at most 7 s.
ENSURE_WAIT = 8
WRITER_WAIT = 2


def write_control(home, disabled):
    data = json.dumps({'v': 1, 'disabled': disabled}, separators=(',', ':')) + '\n'
    home.write(CONTROL, data.encode(), 0o600)


@contextmanager
def locked(home, path, seconds, why):
    """Holds flock on an empty lock file, creating it if need be; refuses after `seconds`."""
    with home.parent(path) as (parent, name):
        fd = os.open(name, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600, dir_fd=parent)
    try:
        deadline = time.monotonic() + seconds
        while True:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise ValueError(why) from None
                time.sleep(.025)
        yield
    finally:
        os.close(fd)


def ensure_locked(home):
    return locked(home, ENSURE_LOCK, ENSURE_WAIT, 'A writer start did not finish; nothing changed')


def enable(home):
    """A fresh start for the writer: the only way out of `disabled` and of the restart ledger."""
    with ensure_locked(home):
        write_control(home, False)
        home.remove(START)
    home.touch(WRITER_LOCK)


@contextmanager
def stopped(home):
    """Disables the writer and holds both locks, so that none runs or starts while the block runs.

    Holding ensure.lock closes the gap where `--ensure` has read `disabled: false` and has not
    spawned yet. If the writer stays past the wait, `disabled` stays set and nothing else changes.
    """
    with ensure_locked(home):
        write_control(home, True)
        why = f'The feed writer did not stop in {WRITER_WAIT} s; it stays disabled and nothing was removed'
        with locked(home, WRITER_LOCK, WRITER_WAIT, why):
            yield


def erase(home):
    """Removes the runtime files, the feed with its question text first. Unknown files stay."""
    if home.kind(DATA) != 'dir':
        return
    staged = [name for name in home.list(DATA) if name.startswith('.herdr-inbox-')]
    for name in (*FILES, *staged):
        home.remove(f'{DATA}/{name}')
