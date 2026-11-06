"""Installs the herdr-app mods into an app home, upgrades them in place, and removes them."""
import argparse
import fcntl
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import layouts
import package
import receipt
import runtime
from check import check_payload
from fs import Home, digest
from receipt import DATA

STAGE = DATA + '/stage'


def scan(home, record, mod):
    """(ours, others) among the files in a mod's folder."""
    known = receipt.known(record)
    ours, others = [], []
    for path in home.walk(f'mods/{mod}'):
        stored = home.read(path) if home.kind(path) == 'file' else None
        host_file = path.count('/') == 2 and path.rsplit('/', 1)[1] in package.HOST_FILES
        (ours if stored and (host_file or digest(stored[0]) in known.get(path, ())) else others).append(path)
    return ours, others


def installed_mods(home, record):
    return [mod for mod in package.mods(receipt.known(record)) if home.kind(f'mods/{mod}') == 'dir']


def preflight(home, payload, old):
    """Refuses anything that would overwrite what is not ours, before the first write."""
    if old is not None:
        others = [path for mod in installed_mods(home, old) for path in scan(home, old, mod)[1]]
        if others:
            raise ValueError('Edited or added files would be lost, nothing changed: ' + ', '.join(others))
    for mod in package.mods(payload):
        if (old is None or mod not in package.mods(receipt.known(old))) and home.kind(f'mods/{mod}') is not None:
            raise ValueError(f'Name collision: mods/{mod}')
    if old is None and home.kind(DATA) is not None and home.list(DATA):
        raise ValueError(f'Name collision: {DATA}')


def stage(home, payload):
    home.rmtree(STAGE)
    home.mkdir(STAGE, 0o700)
    for path, data in payload.items():
        target = f'{STAGE}/{path.removeprefix("mods/")}'
        for depth in range(len(STAGE.split('/')) + 1, target.count('/') + 1):
            home.mkdir('/'.join(target.split('/')[:depth]))
        home.write(target, data, 0o644)


def erase_mod(home, record, mod):
    """Removes our files from a mod's folder; returns what is left there."""
    ours, others = scan(home, record, mod)
    for path in ours:
        home.remove(path)
    for path in sorted({'/'.join(p.split('/')[:depth]) for p in ours for depth in range(3, p.count('/') + 1)},
                       key=len, reverse=True):
        home.rmdir(path)
    home.rmdir(f'mods/{mod}')
    return others


def erase(home, record):
    """Removes everything the install owns, the record last, so that a crash can be run again."""
    leftovers = [path for mod in installed_mods(home, record) for path in erase_mod(home, record, mod)]
    home.rmtree(STAGE)
    runtime.erase(home)
    home.remove(receipt.RECEIPT)
    for mod in package.mods(receipt.known(record)):
        home.rmdir(f'data/{mod}')
    leftovers += [f'{DATA}/{name}' for name in (home.list(DATA) if home.kind(DATA) == 'dir' else ())]
    for path in reversed(record['dirs']):
        home.rmdir(path)
    return leftovers


def install(home, args):
    root = package.plugin(args.plugin_root)
    payload = package.load(root)
    checker = args.checker or shutil.which('herdr-app') or ''
    check_payload(payload, checker)
    old = receipt.load(home)
    preflight(home, payload, old)
    record = {'v': 1, 'plugin_root': root, 'plugin_copy': not Path(root, '.git').exists(),
              'claude_config': os.environ.get('CLAUDE_CONFIG_DIR') or None,
              'dirs': old['dirs'] if old else [], 'previous': {},
              'files': {path: digest(data) for path, data in payload.items()}}
    if old:
        record['previous'] = {path: sorted(digests) for path, digests in receipt.known(old).items()}
    try:
        record['dirs'] += [path for path in ('mods', 'data') if home.mkdir(path)]
        home.mkdir(DATA, 0o700)
        home.chmod(DATA, 0o700)
        stage(home, payload)
        receipt.save(home, record)  # a journal from here on: a crash can be run again or uninstalled
        runtime.enable(home)
        for mod in package.mods(payload):
            home.publish(f'{STAGE}/{mod}', f'mods/{mod}')
        for mod in set(installed_mods(home, record)) - set(package.mods(payload)):
            erase_mod(home, record, mod)
            home.rmdir(f'data/{mod}')
        record['previous'] = {}
        receipt.save(home, record)
        home.rmtree(STAGE)
    except BaseException:
        if old is None:
            erase(home, record)
        raise
    print(f"{'Upgraded' if old else 'Installed'} {', '.join(package.mods(payload))}. No layout, key or setting changed.")
    print('''Open it: herdr-app do mod.open '{"name":"herdr-inbox"}', or bind that in keys.json.''')


def uninstall(home):
    record = receipt.load(home)
    if record is None:
        raise ValueError('Nothing is installed here')
    mods = package.mods(receipt.known(record))
    using = layouts.using(home, mods)
    if layouts.shown(home) in using:
        raise ValueError(f'Layout {layouts.shown(home)} is showing {mods[0]}: switch away from it first; nothing changed')
    with runtime.stopped(home):
        leftovers = erase(home, record)
    for path in leftovers:
        print(f'Kept, not ours: {path}')
    for name in using:
        print(f'Layout {name} still names a removed mod.')
    print('Uninstalled.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('install', 'uninstall'))
    parser.add_argument('--app-home', required=True, help='absolute, existing app home')
    parser.add_argument('--plugin-root', default=str(Path(__file__).resolve().parents[2]),
                        help='the plugin to install from and to run (default: this one)')
    parser.add_argument('--checker', help='absolute herdr-app (default: the one on PATH)')
    args = parser.parse_args()
    try:
        home = Home(args.app_home)
        try:
            try:
                fcntl.flock(home.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise ValueError('Another install is running in this app home') from None
            install(home, args) if args.action == 'install' else uninstall(home)
        finally:
            os.close(home.fd)
        return 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
        print(f'herdr-inbox app {args.action}: {error}', file=sys.stderr)
        return 1
