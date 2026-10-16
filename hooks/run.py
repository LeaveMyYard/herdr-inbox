"""Claude Code hook entry point: stdlib only, always exits 0.

hooks.json runs this as ``sh -c 'python3 -I run.py <event> 2>/dev/null; exit 0'``. Claude
Code gives exit code 2 a meaning of its own: for SessionStart/SubagentStart it shows a
"hook error" notice and the hook's additionalContext (the inbox rules) is lost. ``uv run``
exits 2 on its own failures, e.g. two plugin versions sharing ${CLAUDE_PLUGIN_DATA}/venv
during an update. So hooks import the package straight from the plugin's ``src`` with the
system python3 (``-I``: no user site, no PYTHONPATH, no cwd on sys.path), and nothing here
can turn into a non-zero exit. Only a python3 older than the package needs (3.11) hands
over to uv, with this version's own venv, and still exits 0.

Kept parsable by old Python 3 (no walrus, no match), so the fallback check runs there too.
"""

import os
import signal
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MIN_PYTHON = (3, 11)
FALLBACK_TIMEOUT = 18  # under Claude Code's 20 s hook timeout


def _version():
    try:
        with open(os.path.join(ROOT, "pyproject.toml"), encoding="utf-8") as fh:
            for line in fh:
                if line.startswith("version = "):
                    return line.split('"')[1]
    except (OSError, IndexError):
        pass
    return "unknown"


def _via_uv(event, payload):
    """Old python3: run the hook through uv with this version's venv; forward its output
    only if it succeeded."""
    env = dict(os.environ)
    data = env.get("CLAUDE_PLUGIN_DATA")
    if data:
        env["UV_PROJECT_ENVIRONMENT"] = os.path.join(data, "venv-" + _version())
    try:
        # its own process group: on timeout the whole group goes, so a grandchild still
        # holding stdout cannot keep us waiting on the pipe
        proc = subprocess.Popen(
            ["uv", "run", "--quiet", "--frozen", "--no-dev", "--project", ROOT, "inbox", "hook", event],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            universal_newlines=True, env=env, start_new_session=True)
    except Exception:  # noqa: BLE001
        return
    try:
        out, _ = proc.communicate(payload, timeout=FALLBACK_TIMEOUT)
    except Exception:  # noqa: BLE001 - timeout (or anything else): kill the group, print nothing
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except Exception:  # noqa: BLE001
            pass
        return
    if proc.returncode == 0 and out:
        sys.stdout.write(out)


def main(argv):
    try:
        if len(argv) != 1 or argv[0] not in ("session-start", "subagent-start", "post-tool-batch", "stop"):
            return 0
        if tuple(sys.version_info[:2]) < MIN_PYTHON:
            _via_uv(argv[0], sys.stdin.read())
            return 0
        sys.path.insert(0, os.path.join(ROOT, "src"))
        from herdr_inbox.hook import run  # stdlib-only import chain (tests/test_hook_launch.py)

        run(argv[0])
    except BaseException:  # noqa: BLE001 - a hook must never fail the session
        pass
    return 0


if __name__ == "__main__":
    main(sys.argv[1:])
    try:
        sys.stdout.flush()
    except BaseException:  # noqa: BLE001
        pass
    os._exit(0)
