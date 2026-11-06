"""What ships: the plugin root that is recorded, and the mod files that are installed."""
import json
import os
import re
from pathlib import Path

MOD_NAME = re.compile(r'herdr-inbox(-[a-z0-9]+)*')
PLUGIN_ID = 'id = "leavemyyard.herdr-inbox"'
# Where herdr keeps what `plugin install` fetched. Each update is a new generation, a git checkout
# too, and the old one is deleted, so a `.git` folder does not say that a root stays.
MANAGED = '/plugins/github-installations/'
# herdr-app writes these into a mod's folder: they are not the package's.
HOST_FILES = frozenset(('gpui-kit.d.ts', 'jsconfig.json'))


def plugin(root):
    """The absolute, checked plugin root, as the mods will run its launcher."""
    root = Path(root)
    if not root.is_absolute():
        raise ValueError('--plugin-root must be absolute')
    try:
        identified = PLUGIN_ID in (root / 'herdr-plugin.toml').read_text().splitlines()
    except OSError:
        identified = False
    if not identified or not (root / 'scripts/launch').is_file():
        raise ValueError(f'Not the inbox plugin: {root}')
    return str(root)


def moves(root):
    """Whether an update can move this root. Only a checkout of the operator's own stays where it is."""
    return MANAGED in root + '/' or not Path(root, '.git').exists()


def claude_config():
    """CLAUDE_CONFIG_DIR, when it is a path the mods can use."""
    value = os.environ.get('CLAUDE_CONFIG_DIR', '')
    return value if value.startswith('/') else None


def load(root):
    """{'mods/<mod>/<file>': bytes} for everything under <root>/app/mods."""
    files = {}
    mods_dir = Path(root) / 'app/mods'
    for mod in sorted(mods_dir.iterdir()):
        if not MOD_NAME.fullmatch(mod.name) or mod.is_symlink() or not mod.is_dir():
            raise ValueError(f'Not a herdr-inbox mod folder: {mod.name}')
        for source in sorted(mod.rglob('*')):
            if source.is_symlink():
                raise ValueError(f'A package file is a link: {source}')
            if source.is_file() and not (source.parent == mod and source.name in HOST_FILES):
                files[source.relative_to(mods_dir.parent).as_posix()] = source.read_bytes()
        if not (mod / 'main.js').is_file():
            raise ValueError(f'{mod.name} has no main.js')
        if json.loads((mod / 'mod.json').read_text()).get('place') != 'none':
            raise ValueError(f'{mod.name} must declare place none')
    if not files:
        raise ValueError('The package has no mods')
    return files


def mods(paths):
    """The mod names in these 'mods/<mod>/...' paths."""
    return sorted({path.split('/')[1] for path in paths})
