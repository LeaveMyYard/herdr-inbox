"""Whether a mod can go. Removing a mod the shown layout uses leaves a hole in the window.

Only the window knows what it shows: it can keep a layout whose file has since been renamed or
broken. So it is asked, and an answer that can't be used refuses. Everything here only reads, so
links are followed, as the host follows them. What this can't see is a mod opened in a tab: `ls`
and `status` don't name it. There, and for an open overlay, the window reports the missing mod
until the tab or overlay closes."""
import json
import os
import shutil
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


def shown(app_home):
    """The layout on screen; None when no window runs, which makes removing anything safe."""
    program = shutil.which('herdr-app')
    if program is None:
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


def guard(app_home, mods):
    """Refuses when the shown layout uses one of the mods. Returns notes on the other layouts."""
    name = shown(app_home)
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
