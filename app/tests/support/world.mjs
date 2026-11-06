// A private plugin and app home per test. Nothing here reaches the caller's herdr-app: every path
// is under the test's own temporary folder, and the checker is a stand-in.
import { spawnSync } from 'node:child_process';
import { cpSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, readlinkSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const repo = fileURLToPath(new URL('../../..', import.meta.url));

// Refuses what `herdr-app check` must never see, and leaves what the real one leaves.
const CHECKER = `#!/usr/bin/env python3
import os, sys
from pathlib import Path
assert len(sys.argv) == 3 and sys.argv[1] == 'check', sys.argv
leaked = [k for k in os.environ if k.startswith(('HERDR_SOCKET', 'HC_SOCKET', 'CLAUDE_')) or k == 'TEST_SECRET']
assert not leaked, leaked
assert os.environ['HC_SESSION'].startswith('inbox-check-')
scratch = os.environ['HOME']
assert Path(scratch).stat().st_mode & 0o777 == 0o700
for name in ('HERDR_APP_HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_RUNTIME_DIR'):
    assert os.environ[name].startswith(scratch + '/'), name
assert sys.argv[2].startswith(scratch + '/')
log = Path(sys.argv[0]).with_suffix('.log')
with log.open('a') as stream:
    stream.write(sys.argv[2] + '\\n')
Path(sys.argv[2], 'gpui-kit.d.ts').write_text('// generated')
if Path(sys.argv[0]).with_suffix('.socket').exists():
    import socket
    folder = Path(os.environ['XDG_RUNTIME_DIR'], 'herdr-core')
    folder.mkdir()
    socket.socket(socket.AF_UNIX).bind(str(folder / (os.environ['HC_SESSION'] + '.sock')))
if Path(sys.argv[0]).with_suffix('.fail').exists():
    print('Question Title 123 failed to load', file=sys.stderr)
    sys.exit(9)
`;

// What the tests' PATH finds first, so that no test can reach a real one. `herdr-app status` answers
// what `window` set, or no window. `hc` must never run at all.
const HERDR_APP = `#!/bin/sh
here=$(dirname "$0")
[ "$1" = status ] || exit 64
echo "$HERDR_APP_HOME" >> "$here/status.log"
[ -f "$here/status.delay" ] && sleep "$(cat "$here/status.delay")"
[ -f "$here/status.out" ] && cat "$here/status.out"
exit "$(cat "$here/status.code" 2>/dev/null || echo 2)"
`;
const HC = `#!/bin/sh
echo "$@" >> "$(dirname "$0")/hc.log"
exit 1
`;

export function sandbox(t, { checkout = false, root = 'plugin' } = {}) {
  const temp = realpathSync(mkdtempSync(join(tmpdir(), 'inbox-app-')));
  t.after(() => rmSync(temp, { recursive: true, force: true }));
  const home = join(temp, 'home');
  const plugin = join(temp, root);
  const tools = join(temp, 'tools');
  const checker = join(temp, 'checker');
  mkdirSync(home);
  mkdirSync(join(plugin, 'scripts'), { recursive: true });
  cpSync(join(repo, 'app/mods'), join(plugin, 'app/mods'), { recursive: true });
  cpSync(join(repo, 'herdr-plugin.toml'), join(plugin, 'herdr-plugin.toml'));
  cpSync(join(repo, 'scripts/launch'), join(plugin, 'scripts/launch'));
  if (checkout) mkdirSync(join(plugin, '.git'));
  writeFileSync(checker, CHECKER, { mode: 0o755 });
  mkdirSync(tools);
  for (const [name, text] of [['herdr-app', HERDR_APP], ['hc', HC]]) writeFileSync(join(tools, name), text, { mode: 0o755 });
  const env = { ...process.env, PATH: `${tools}:${process.env.PATH}`, TEST_SECRET: 'x', HERDR_SOCKET_PATH: '/fake/live.sock', HC_SOCKET: '/fake/core.sock' };
  delete env.CLAUDE_CONFIG_DIR;
  const run = (action, { root = plugin, extra = {} } = {}) => spawnSync('python3', [
    join(repo, 'scripts/app-install'), action, '--app-home', home, '--plugin-root', root,
    ...(action === 'install' ? ['--checker', checker] : []),
  ], { env: { ...env, ...extra }, encoding: 'utf8' });
  const inHome = path => join(home, path);
  const receipt = () => JSON.parse(readFileSync(inHome('data/herdr-inbox/install.json'), 'utf8'));
  const window = (out, code = 0) => {
    writeFileSync(join(tools, 'status.out'), out);
    writeFileSync(join(tools, 'status.code'), String(code));
  };
  return { temp, home, plugin, checker, tools, env, run, inHome, receipt, window };
}

// Every path under a folder with its bytes and mode, to compare a home before and after.
export function tree(directory) {
  const found = {};
  const walk = (dir, prefix) => {
    for (const name of readdirSync(dir).sort()) {
      const path = join(dir, name);
      const key = prefix + name;
      const info = lstatSync(path);
      if (info.isSymbolicLink()) found[key] = { link: readlinkSync(path) };
      else if (info.isDirectory()) {
        found[`${key}/`] = { mode: info.mode & 0o777 };
        walk(path, `${key}/`);
      } else found[key] = { data: readFileSync(path).toString('base64'), mode: info.mode & 0o777 };
    }
  };
  walk(directory, '');
  return found;
}

// A home with an operator's own files in it, and Panorama's mods and data next to ours.
export function lived(s) {
  const files = {
    'keys.json': '{ "global": { "alt-p": "personal.command" } }\n',
    'settings.json': '{ "zoom": 1.0 }\n',
    'shared.json': '{"layout":"main"}',
    'view.json': '{"active":null}\n',
    'layouts/main.json': '{ "content": { "mod": "tabs" } }\n',
    'layouts/project-alpha.json': '{ "content": { "mod": "panorama" } }\n',
    'mods/personal/main.js': '// mine\n',
    'mods/panorama/main.js': '// panorama\n',
    'mods/panorama/mod.json': '{ "place": "none" }\n',
    'mods/panorama-peek/main.js': '// peek\n',
    'data/panorama/install.json': '{}\n',
    'data/panorama/feed.json': '{"v":1}\n',
    'data/panorama-spaces/store.json': '{}\n',
  };
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(join(s.home, path, '..'), { recursive: true });
    writeFileSync(join(s.home, path), text);
  }
}
