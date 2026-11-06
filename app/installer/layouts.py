"""Whether a mod can go. Removing a mod the shown layout uses leaves a hole in the window.

Only the window knows what it shows: it can keep a layout whose file has since been renamed or
broken. So it is asked, and an answer that can't be used refuses. Everything here only reads, so
links are followed, as the host follows them. What this can't see is a mod opened in a tab: `ls`
and `status` don't name it. There, and for an open overlay, the window reports the missing mod
until the tab or overlay closes."""
import errno
import json
import os
import socket
import subprocess
from pathlib import Path

# herdr-app: "no Herdr window".
NO_WINDOW = 2


def names(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from names(item)
    elif isinstance(value, list):
        for item in value:
            yield from names(item)


def window_runs(app_home):
    """With no program to ask, the window's own socket says: only absent or refused mean no window."""
    with socket.socket(socket.AF_UNIX) as probe:
        try:
            probe.connect(f'{app_home}/herdr-app.sock')
        except OSError as error:
            return error.errno not in (errno.ENOENT, errno.ECONNREFUSED)
    return True


def shown(app_home, program):
    """The layout on screen; None when no window runs, which makes removing anything safe."""
    if not program:
        if window_runs(app_home):
            raise ValueError('A window may be running and herdr-app was not found, so nothing can say what it '
                             'shows; nothing changed. Quit it, or pass --checker with the path to herdr-app')
        return None
    env = {**os.environ, 'HERDR_APP_HOME': app_home}
    try:
        result = subprocess.run([program, 'status'], env=env, capture_output=True, timeout=3)
    except (OSError, subprocess.TimeoutExpired):
        raise ValueError('The window did not say what it shows; nothing changed. Quit it and try again') from None
    if result.returncode == NO_WINDOW:
        return None
    try:
        name = json.loads(result.stdout)['layout'] if result.returncode == 0 else None
    except (ValueError, KeyError, TypeError):
        name = None
    if not isinstance(name, str) or not name or '/' in name:
        raise ValueError('The window did not say what it shows; nothing changed. Quit it and try again')
    return name


def named(path):
    """Every string in a layout file. Raises when it can't be read."""
    return set(names(json.loads(Path(path).read_bytes())))


def guard(app_home, mods, program):
    """Refuses when the shown layout uses one of the mods. Returns notes on the other layouts."""
    name = shown(app_home, program)
    if name is not None:
        try:
            used = sorted(set(mods) & named(f'{app_home}/layouts/{name}.json'))
        except (OSError, ValueError):
            raise ValueError(f'Layout {name} is on screen and its file cannot be read, so it may show '
                             f'{mods[0]}; nothing changed. Switch layouts or quit the window') from None
        if used:
            raise ValueError(f'Layout {name} is showing {used[0]}; nothing changed. Switch away from it first')
    notes = []
    for path in sorted(Path(app_home, 'layouts').glob('*.json')):
        try:
            if set(mods) & named(path):
                notes.append(f'Layout {path.stem} still names a removed mod.')
        except (OSError, ValueError):
            pass
    return notes
