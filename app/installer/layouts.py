"""Which layouts show a mod. Removing a mod the shown layout uses would leave a hole in the window."""
import json
import re

LIMIT = 8 * 1024 * 1024
NAME = re.compile(r'[A-Za-z0-9._-]+\.json')


def names(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from names(item)
    elif isinstance(value, list):
        for item in value:
            yield from names(item)


def parsed(home, path):
    """The JSON at path, or None: a personal file we cannot read says nothing about our mods."""
    try:
        stored = home.read(path, LIMIT)
        return None if stored is None else json.loads(stored[0])
    except (OSError, ValueError):
        return None


def using(home, mods):
    """The layouts, by file name, that name one of the mods."""
    if home.kind('layouts') != 'dir':
        return []
    files = [name for name in home.list('layouts') if NAME.fullmatch(name)]
    return [name[:-5] for name in files if set(mods) & set(names(parsed(home, f'layouts/{name}')))]


def shown(home):
    value = parsed(home, 'shared.json')
    return value.get('layout') if isinstance(value, dict) else None
