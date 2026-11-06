"""`herdr-app check` on exactly what will be installed, in a private home.

A check runs the mod's `init` and what that spawns, so nothing of the caller's reaches it: no
HERDR_*, HC_* or CLAUDE_*, and a home of its own. It needs no compositor."""
import os
import shutil
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from package import mods

PASSED_ON = ('PATH', 'LANG', 'LC_ALL')


def sockets(scratch):
    return [path for path in Path(scratch).rglob('*') if path.is_socket()]


def stop_core(scratch, env):
    """A core a check started answers on a socket in scratch, so this reaches no other."""
    # `hc server status` starts a core that is missing; `stop` is only called on a socket we saw.
    hc = shutil.which('hc', path=env['PATH'])
    if hc:
        subprocess.run([hc, 'server', 'stop', '--json'], env=env, capture_output=True, timeout=5)
    deadline = time.monotonic() + 3
    while sockets(scratch) and time.monotonic() < deadline:
        time.sleep(.025)
    if sockets(scratch):
        raise ValueError(f'A core the check started did not stop; its files are kept in {scratch}')


def check_payload(payload, checker):
    if not os.path.isabs(checker) or not Path(checker).is_file():
        raise ValueError('herdr-app is required: --checker must name an absolute executable')
    # Unix socket paths are short, and macOS's default temporary folder is already deep.
    scratch = tempfile.mkdtemp(prefix='hi-', dir='/tmp')
    env = {name: os.environ[name] for name in PASSED_ON if name in os.environ}
    env.update(HOME=scratch, HERDR_APP_HOME=f'{scratch}/app', HC_SESSION=f'inbox-check-{uuid.uuid4().hex[:12]}',
               XDG_CONFIG_HOME=f'{scratch}/config', XDG_STATE_HOME=f'{scratch}/state',
               XDG_RUNTIME_DIR=f'{scratch}/runtime', HERDR_TELEMETRY='0', DO_NOT_TRACK='1')
    for name in ('app', 'config', 'state', 'runtime'):
        Path(scratch, name).mkdir(mode=0o700)
    for path, data in payload.items():
        target = Path(scratch, path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    try:
        for mod in mods(payload):
            result = subprocess.run([checker, 'check', f'{scratch}/mods/{mod}'], env=env, capture_output=True, timeout=30)
            if result.returncode:
                output = (result.stderr or result.stdout).decode(errors='replace').strip()
                raise ValueError(f'herdr-app check failed for {mod}: {output}')
    finally:
        if sockets(scratch):
            stop_core(scratch, env)
        shutil.rmtree(scratch)
