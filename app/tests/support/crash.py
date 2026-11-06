"""Stops the real installer dead at every call that changes the disk, and checks what is left.

For each point: the mod folder must be whole, and running install or uninstall again must end where
a run with no crash ends. os._exit runs no cleanup, as a power cut or SIGKILL would not.
Usage: crash.py <repo> <plugin v1> <plugin v2> <work folder>. Prints JSON."""
import json
import os
import shutil
import sys
from pathlib import Path

sys.dont_write_bytecode = True
repo, plugin_a, plugin_b, work = sys.argv[1:5]
sys.path.insert(0, f'{repo}/app/installer')
import atomic
import main

CRASHED = 99
PATCHED = ('mkdir', 'replace', 'unlink', 'rmdir', 'fsync', 'fchmod')


def snapshot(home):
    found = {}
    for path in sorted(Path(home).rglob('*')):
        relative = str(path.relative_to(home))
        found[relative] = None if path.is_dir() else path.read_bytes().hex()
    return found


def mod_files(plugin):
    folder = Path(plugin, 'app/mods/herdr-inbox')
    return {path.relative_to(folder).as_posix(): path.read_bytes().hex() for path in folder.rglob('*') if path.is_file()}


def installed(home):
    folder = Path(home, 'mods/herdr-inbox')
    if not folder.exists():
        return None
    return {path.relative_to(folder).as_posix(): path.read_bytes().hex() for path in folder.rglob('*')
            if path.is_file() and path.name not in ('gpui-kit.d.ts', 'jsconfig.json')}


def arm(crash_at):
    count = [0]

    def tick():
        if count[0] == crash_at:
            os._exit(CRASHED)
        count[0] += 1

    def wrap(function):
        def wrapped(*args, **kwargs):
            tick()
            return function(*args, **kwargs)
        return wrapped

    for name in PATCHED:
        setattr(os, name, wrap(getattr(os, name)))
    atomic._rename = wrap(atomic._rename)
    real_open = os.open

    def opened(path, flags, *args, **kwargs):
        if flags & os.O_CREAT:
            tick()
        return real_open(path, flags, *args, **kwargs)

    os.open = opened


def run(action, home, plugin=plugin_a, crash_at=None):
    """The exit status of one real installer run, in a process of its own."""
    pid = os.fork()
    if pid == 0:
        null = os.open(os.devnull, os.O_WRONLY)
        os.dup2(null, 1)
        os.dup2(null, 2)
        main.check_payload = lambda *_: None
        if crash_at is not None:
            arm(crash_at)
        sys.argv = ['app-install', action, '--app-home', home, '--plugin-root', plugin]
        os._exit(main.main())
    return os.waitstatus_to_exitcode(os.waitpid(pid, 0)[1])


def fresh(source, name):
    target = f'{work}/{name}'
    shutil.rmtree(target, ignore_errors=True)
    shutil.copytree(source, target, symlinks=True)
    return target


def only_empty_data(tree, before):
    """A crash between `mkdir data` and the next write leaves one empty folder and nothing to say it is ours."""
    return {k: v for k, v in tree.items() if k != 'data' or 'data' in before} == before


def sweep(name, base, action, plugin, whole, expected_home, failures):
    """Crashes `action` on a copy of `base` at each point in turn; returns how many points there were."""
    point = 0
    while True:
        home = fresh(base, 'crashed')
        status = run(action, home, plugin, point)
        if status != CRASHED:
            return point
        where = f'{name}@{point}'
        if installed(home) is not None and installed(home) not in whole:
            failures.append(f'{where}: the mod folder is partial')
        # install again finishes it, and uninstall then leaves the home as it was
        again = fresh(home, 'again')
        status = run('install', again, plugin_b if plugin == plugin_b else plugin_a)
        if status != 0:
            failures.append(f'{where}: install after the crash exited {status}')
        elif installed(again) != mod_files(plugin_b if plugin == plugin_b else plugin_a):
            failures.append(f'{where}: install after the crash left stale files')
        elif json.loads(Path(again, 'data/herdr-inbox/install.json').read_text())['previous']:
            failures.append(f'{where}: install after the crash left a journal')
        elif any(name.startswith('.herdr-inbox-') for name in os.listdir(f'{again}/data/herdr-inbox')):
            failures.append(f'{where}: install after the crash kept a staged file')
        else:
            status = run('uninstall', again)
            if status != 0 or snapshot(again) != expected_home:
                failures.append(f'{where}: uninstall after install did not restore the home')
        # or uninstall takes everything out at once
        again = fresh(home, 'again')
        status = run('uninstall', again)
        if not only_empty_data(snapshot(again), expected_home):
            failures.append(f'{where}: uninstall after the crash exited {status} and left {sorted(set(snapshot(again)) - set(expected_home))}')
        point += 1


def twice(base, expected_home, failures):
    """Two upgrades crash in a row with the mods still at the first version: its digests must last."""
    third = fresh(plugin_b, 'plugin-c')
    Path(third, 'app/mods/herdr-inbox/third.js').write_text('// third\n')
    record = Path('data/herdr-inbox/install.json')
    point = 0
    while True:
        home = fresh(base, 'crashed')
        if run('install', home, plugin_b, point) != CRASHED:
            failures.append('twice: no point had the journal written and the mods not swapped')
            return
        if json.loads(Path(home, record).read_text())['previous'] and installed(home) == mod_files(plugin_a):
            break
        point += 1
    if run('install', home, third, point) != CRASHED:
        failures.append('twice: the second upgrade did not reach the same point')
    elif run('install', home, third) != 0 or installed(home) != mod_files(third):
        failures.append('twice: the install after two crashed upgrades failed')
    elif run('uninstall', home) != 0 or snapshot(home) != expected_home:
        failures.append('twice: the uninstall after two crashed upgrades did not restore the home')


def lived(home):
    for path, text in (('settings.json', '{}\n'), ('mods/panorama/main.js', '// panorama\n'), ('data/panorama/feed.json', '{}\n')):
        Path(home, path).parent.mkdir(parents=True, exist_ok=True)
        Path(home, path).write_text(text)


def runtime_files(home):
    for name, text in (('feed.json', '{"v":1}\n'), ('writer.json', '{}\n'), ('start.json', '{}\n')):
        Path(home, 'data/herdr-inbox', name).write_text(text)
    Path(home, 'mods/herdr-inbox/gpui-kit.d.ts').write_text('// host\n')


def scenario(label, prepare, failures, points):
    empty = f'{work}/{label}-empty'
    os.mkdir(empty)
    prepare(empty)
    before = snapshot(empty)
    points[f'{label} first install'] = sweep(f'{label} first', empty, 'install', plugin_a, [mod_files(plugin_a)], before, failures)
    one = fresh(empty, f'{label}-one')
    assert run('install', one) == 0
    runtime_files(one)
    whole = [mod_files(plugin_a), mod_files(plugin_b)]
    points[f'{label} upgrade'] = sweep(f'{label} upgrade', one, 'install', plugin_b, whole, before, failures)
    twice(one, before, failures)
    point = 0
    while True:
        home = fresh(one, 'crashed')
        if run('uninstall', home, crash_at=point) != CRASHED:
            break
        status = run('uninstall', home)
        if not only_empty_data(snapshot(home), before):
            failures.append(f'{label} uninstall@{point}: running it again exited {status} and left {sorted(set(snapshot(home)) - set(before))}')
        point += 1
    points[f'{label} uninstall'] = point


failures, points = [], {}
scenario('empty', lambda home: None, failures, points)
scenario('lived', lived, failures, points)
print(json.dumps({'points': points, 'failures': failures}))
