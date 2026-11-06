"""`herdr-app check` on exactly what will be installed, in a private home.

A check runs the mod's `init` and what that spawns, so nothing of the caller's reaches it: no
HERDR_*, HC_* or CLAUDE_*, and a home of its own. It needs no compositor.

hc falls back to the user's own runtime folder when XDG_RUNTIME_DIR is unset, so setting that
variable is what keeps a check off the operator's core. Nothing here runs `hc` or signals a process:
`hc server status` can't name the socket it reached, so nothing could prove a pid is the private core's.

The rule for mods: nothing calls `hc` from `init`. A mod that does starts a private core, which stays
running, one per check. A pass is no proof it didn't: the check only sees a socket that appears
within SETTLE of the end, so the same bad mod can pass, and a core started later goes unseen."""
import os
import shutil
import socket
import struct
import subprocess
import sys
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
    # What protects is XDG_RUNTIME_DIR above. This only stops a scratch folder in the real runtime folder.
    if scratch.startswith('/run/user/'):
        raise ValueError(f'A check must not reach the default core socket: {scratch}')
    return env


def owner(path):
    """The pid listening on a socket, from the kernel, or None. Connecting sends nothing."""
    try:
        with socket.socket(socket.AF_UNIX) as peer:
            peer.settimeout(1)
            peer.connect(path)
            if sys.platform == 'darwin':  # SOL_LOCAL, LOCAL_PEERPID
                return struct.unpack('i', peer.getsockopt(0, 2, 4))[0]
            return struct.unpack('3i', peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0]
    except (OSError, AttributeError):
        return None


def refuse_server(path, failure):
    pid = owner(path)
    who = f'pid {pid}' if pid else 'its pid is unknown'
    ending = f' The check also failed: {failure}' if failure else ''
    raise ValueError(f'A check started a server, which a mod must not do from init. It is private and still '
                     f'running: {who}, socket {path}. Stop that pid; do not run hc server stop by hand.{ending}')


def check_payload(payload, checker):
    if not os.path.isabs(checker) or not Path(checker).is_file():
        raise ValueError('herdr-app is required: --checker must name an absolute executable')
    # Unix socket paths are short, and macOS's default temporary folder is already deep.
    scratch = tempfile.mkdtemp(prefix='hi-', dir='/tmp')
    failure = None
    try:
        env = private_env(scratch)
        for name in ('app', 'config', 'state', 'data', 'cache', 'runtime'):
            Path(scratch, name).mkdir(mode=0o700)
        for path, data in payload.items():
            target = Path(scratch, path)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        for mod in mods(payload):
            result = subprocess.run([checker, 'check', f'{scratch}/mods/{mod}'], env=env, capture_output=True, timeout=30)
            if result.returncode:
                output = (result.stderr or result.stdout).decode(errors='replace').strip()
                raise ValueError(f'herdr-app check failed for {mod}: {output}')
    except BaseException as error:
        failure = error
        raise
    finally:
        time.sleep(SETTLE)
        left = sockets(scratch)
        if left:
            refuse_server(left[0], failure)
        shutil.rmtree(scratch)
