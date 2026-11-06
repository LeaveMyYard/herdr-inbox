"""The install record. It alone is the authority for what uninstall removes, so uninstall needs
no package, and it is checked so that an edited record can name nothing outside our own folders."""
import json
import re

from fs import parts

DATA = 'data/herdr-inbox'
RECEIPT = DATA + '/install.json'
LIMIT = 1024 * 1024
FIELDS = {'v', 'plugin_root', 'plugin_moves', 'claude_config', 'dirs', 'files', 'previous'}
PATH = re.compile(r'mods/herdr-inbox(-[a-z0-9]+)*/.+')
SHA256 = re.compile(r'[a-f0-9]{64}')


def owned_path(path):
    if not isinstance(path, str) or not PATH.fullmatch(path):
        return False
    try:
        return bool(parts(path))
    except ValueError:
        return False


def absolute(value):
    return isinstance(value, str) and value.startswith('/')


def digests(value):
    return isinstance(value, list) and all(isinstance(d, str) and SHA256.fullmatch(d) for d in value)


def validate(value):
    if not isinstance(value, dict) or set(value) != FIELDS or value['v'] != 1:
        raise ValueError('Invalid install record')
    if (not absolute(value['plugin_root']) or type(value['plugin_moves']) is not bool
            or not (value['claude_config'] is None or absolute(value['claude_config']))):
        raise ValueError('Invalid plugin location in the install record')
    dirs, files, previous = value['dirs'], value['files'], value['previous']
    if not isinstance(dirs, list) or not set(dirs) <= {'mods', 'data'}:
        raise ValueError('Invalid directories in the install record')
    if (not isinstance(files, dict) or not isinstance(previous, dict)
            or not all(owned_path(path) and digests([d]) for path, d in files.items())
            or not all(owned_path(path) and digests(d) for path, d in previous.items())):
        raise ValueError('The install record names a file that is not ours')
    return value


def load(home):
    """The checked record, or None when nothing is installed."""
    stored = home.read(RECEIPT, LIMIT)
    if stored is None:
        return None
    try:
        if stored[1] != 0o600:
            raise ValueError('it must have mode 0600')
        return validate(json.loads(stored[0]))
    except (ValueError, TypeError) as error:
        raise ValueError(f'{RECEIPT} cannot be used: {error}. Nothing changed. If it is lost, remove '
                         f'mods/herdr-inbox* and {DATA} by hand and install again') from None


def save(home, record):
    home.write(RECEIPT, (json.dumps(record, indent=2) + '\n').encode(), 0o600)


def known(record):
    """{path: digests} for every file of this generation or of an upgrade that did not finish."""
    result = {path: set(digests_) for path, digests_ in record['previous'].items()}
    for path, sha256 in record['files'].items():
        result.setdefault(path, set()).add(sha256)
    return result
