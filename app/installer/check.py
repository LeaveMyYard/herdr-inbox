"""`herdr-app check` on exactly what will be installed, in a private home.

A check runs the mod's `init` and what that spawns, so nothing of the caller's reaches it: no
HERDR_*, HC_* or CLAUDE_*, and a home of its own. It needs no compositor.

hc falls back to the user's own runtime folder when XDG_RUNTIME_DIR is unset, so that variable is
what keeps a check off the operator's core. Nothing here runs `hc` or signals a process: `hc server
status` can't name the socket it reached, so nothing could prove a pid is the private core's."""
import os
import shutil
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from package import mods

PASSED_ON = ('PATH', 'LANG', 'LC_ALL')
# A mod's task can start a core just after the check returns.
SETTLE = .3


def sockets(scratch):
    return [str(path) for path in Path(scratch).rglob('*') if path.is_socket()]


def private_env(scratch):
    env = {name: os.environ[name] for name in PASSED_ON if name in os.environ}
    env.update(HOME=scratch, HERDR_APP_HOME=f'{scratch}/app', HC_SESSION=f'inbox-check-{uuid.uuid4().hex[:12]}',
               XDG_CONFIG_HOME=f'{scratch}/config', XDG_STATE_HOME=f'{scratch}/state', XDG_DATA_HOME=f'{scratch}/data',
               XDG_CACHE_HOME=f'{scratch}/cache', XDG_RUNTIME_DIR=f'{scratch}/runtime',
               HERDR_TELEMETRY='0', DO_NOT_TRACK='1')
    socket = f"{env['XDG_RUNTIME_DIR']}/herdr-core/{env['HC_SESSION']}.sock"
    if not socket.startswith(scratch + '/') or socket.startswith('/run/user/'):
        raise ValueError(f'A check must not reach the default core socket: {socket}')
    return env


def check_payload(payload, checker):
    if not os.path.isabs(checker) or not Path(checker).is_file():
        raise ValueError('herdr-app is required: --checker must name an absolute executable')
    # Unix socket paths are short, and macOS's default temporary folder is already deep.
    scratch = tempfile.mkdtemp(prefix='hi-', dir='/tmp')
    env = private_env(scratch)
    for name in ('app', 'config', 'state', 'data', 'cache', 'runtime'):
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
        time.sleep(SETTLE)
        left = sockets(scratch)
        if left:
            raise ValueError(f'A check started a server, which a mod must not do from init. It is private; '
                             f'it stays, with its files, at {left[0]}')
        shutil.rmtree(scratch)
