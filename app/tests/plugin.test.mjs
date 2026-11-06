import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, cpSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { availability, launcher } from '../mods/herdr-inbox/plugin.js';
import { repo, sandbox } from './support/world.mjs';

const MANIFEST = 'id = "leavemyyard.herdr-inbox"\nversion = "1.0.0"\n';
const RECEIPT = '/fake/app/data/herdr-inbox/install.json';
const CLAUDE = '/fake/home/.claude/plugins/installed_plugins.json';
const HELP = { code: 0, stdout: 'herdr-inbox app v1\nusage\n', stderr: '' };

const plugin = root => ({ [`${root}/herdr-plugin.toml`]: MANIFEST, [`${root}/scripts/launch`]: '# launch\n' });
const record = fields => JSON.stringify({ v: 1, plugin_root: '/fake/recorded', plugin_copy: true, claude_config: null, ...fields });
const registry = (installPath, scope = 'user') => JSON.stringify({ version: 2, plugins: { 'herdr-inbox@herdr-inbox': [{ scope, installPath }] } });
const herdrList = root => JSON.stringify({ result: { plugins: [{ plugin_id: 'other', plugin_root: '/fake/other' }, { plugin_id: 'leavemyyard.herdr-inbox', plugin_root: root }] } });

function host({ files = {}, herdr = null, app = HELP }) {
  const calls = [];
  return {
    calls,
    appHome: () => '/fake/app',
    home: () => '/fake/home',
    readTextFile: async path => files[path] ?? null,
    exec: async (command, args) => {
      calls.push([command, ...args]);
      if (command === 'herdr') {
        if (herdr === null) throw Error('not installed');
        return herdr;
      }
      if (app instanceof Error) throw app;
      return app;
    },
  };
}

test('with no install record the inbox is unavailable, and nothing runs', async () => {
  const h = host({});
  assert.deepEqual(await availability(h), { state: 'unavailable', reason: 'not-installed' });
  assert.deepEqual(h.calls, []);
});

test('an unreadable or unknown record reads as not installed', async () => {
  for (const text of ['{', '{"v":2}', 'null']) {
    assert.equal((await availability(host({ files: { [RECEIPT]: text } }))).reason, 'not-installed');
  }
  const throwing = { ...host({}), readTextFile: async () => { throw Error('denied'); } };
  assert.equal((await availability(throwing)).reason, 'not-installed');
});

test('a machine with only herdr-app: no herdr to ask, the recorded root is used', async () => {
  const h = host({ files: { [RECEIPT]: record({}), ...plugin('/fake/recorded') } });
  assert.deepEqual(await availability(h), { state: 'ready', root: '/fake/recorded' });
  assert.deepEqual(h.calls.at(-1), ['sh', '/fake/recorded/scripts/launch', 'cli', 'app', '--help']);
});

test('a copy that moved is found through Claude Code, with no server to ask', async () => {
  const files = { [RECEIPT]: record({}), [CLAUDE]: registry('/fake/new'), ...plugin('/fake/new') };
  assert.deepEqual(await availability(host({ files })), { state: 'ready', root: '/fake/new' });
});

test('the recorded Claude config folder is the one read', async () => {
  const files = { [RECEIPT]: record({ claude_config: '/fake/claude' }), '/fake/claude/plugins/installed_plugins.json': registry('/fake/new'), ...plugin('/fake/new') };
  assert.equal((await availability(host({ files }))).root, '/fake/new');
});

test('a project-scope Claude entry belongs to that project and is not used', async () => {
  const files = { [RECEIPT]: record({}), [CLAUDE]: registry('/fake/new', 'project'), ...plugin('/fake/new'), ...plugin('/fake/recorded') };
  assert.equal((await availability(host({ files }))).root, '/fake/recorded');
});

test('when a herdr server answers, its root comes first, even with the old copy still on disk', async () => {
  const files = { [RECEIPT]: record({}), ...plugin('/fake/recorded'), ...plugin('/fake/herdr'), ...plugin('/fake/claude-copy'), [CLAUDE]: registry('/fake/claude-copy') };
  const h = host({ files, herdr: { code: 0, stdout: herdrList('/fake/herdr'), stderr: '' } });
  assert.equal((await availability(h)).root, '/fake/herdr');
});

test('a herdr that fails or answers nonsense says nothing', async () => {
  for (const herdr of [{ code: 1, stdout: '', stderr: 'no server' }, { code: 0, stdout: 'nonsense', stderr: '' }, { code: 0, stdout: '{"result":{"plugins":{}}}', stderr: '' }]) {
    const files = { [RECEIPT]: record({}), ...plugin('/fake/recorded') };
    assert.equal((await availability(host({ files, herdr }))).root, '/fake/recorded');
  }
});

test('a root that is another plugin, or has no launcher, is passed over', async () => {
  const files = {
    [RECEIPT]: record({}), [CLAUDE]: registry('/fake/panorama'),
    '/fake/panorama/herdr-plugin.toml': 'id = "leavemyyard.herdr-panorama"\n', '/fake/panorama/scripts/launch': '#\n',
    '/fake/recorded/herdr-plugin.toml': MANIFEST,
  };
  assert.deepEqual(await availability(host({ files })), { state: 'unavailable', reason: 'no-plugin' });
});

test('a checkout stays where it is: herdr and Claude Code are not asked', async () => {
  const files = { [RECEIPT]: record({ plugin_copy: false }), ...plugin('/fake/recorded'), [CLAUDE]: registry('/fake/new'), ...plugin('/fake/new') };
  const h = host({ files, herdr: { code: 0, stdout: herdrList('/fake/new'), stderr: '' } });
  assert.equal((await availability(h)).root, '/fake/recorded');
  assert.ok(h.calls.every(([command]) => command !== 'herdr'));
});

test('a checkout that is gone is unavailable, not replaced by the live plugin', async () => {
  const files = { [RECEIPT]: record({ plugin_copy: false }), [CLAUDE]: registry('/fake/new'), ...plugin('/fake/new') };
  assert.deepEqual(await availability(host({ files })), { state: 'unavailable', reason: 'no-plugin' });
});

test('anything but the probe line, exit 0, is unavailable', async () => {
  const answers = [
    { code: 2, stdout: '', stderr: 'usage: inbox' }, // Python: the rollback, or no binary
    { code: 0, stdout: 'herdr-inbox 0.10.6\n', stderr: '' }, // a binary older than `app`
    { code: 0, stdout: '', stderr: '' },
    { code: 1, stdout: 'herdr-inbox app v1\n', stderr: '' },
    { code: 0, stdout: 'herdr-inbox app v2\n', stderr: '' },
    Error('sh failed to start'),
  ];
  for (const app of answers) {
    const files = { [RECEIPT]: record({}), ...plugin('/fake/recorded') };
    assert.deepEqual(await availability(host({ files, app })), { state: 'unavailable', reason: 'no-app' });
  }
});

test('launcher goes through scripts/launch, never a binary of its own', () => {
  assert.deepEqual(launcher('/fake/a b', ['app', 'show', '--json']), ['sh', ['/fake/a b/scripts/launch', 'cli', 'app', 'show', '--json']]);
});

// The real launcher, with a stand-in `uv` for Python. It keeps every path under the test's folder.
function realHost(t, { engine, uv }) {
  const s = sandbox(t);
  cpSync(join(repo, 'scripts/release-lib.sh'), join(s.plugin, 'scripts/release-lib.sh'));
  writeFileSync(join(s.plugin, 'engine'), `${engine}\n`);
  mkdirSync(join(s.temp, 'tools'));
  writeFileSync(join(s.temp, 'tools/uv'), `#!/bin/sh\n${uv}\n`);
  chmodSync(join(s.temp, 'tools/uv'), 0o755);
  const env = { PATH: `${join(s.temp, 'tools')}:/usr/bin:/bin`, HOME: s.temp, XDG_STATE_HOME: join(s.temp, 'state') };
  const files = { [RECEIPT]: record({ plugin_root: s.plugin }), ...plugin(s.plugin) };
  return {
    ...host({ files }),
    exec: async (command, args) => {
      if (command === 'herdr') throw Error('not installed');
      const result = spawnSync(command, args, { env, encoding: 'utf8' });
      return { code: result.status, stdout: result.stdout, stderr: result.stderr };
    },
  };
}

test('engine rust with no binary falls to Python, so the inbox says unavailable', async t => {
  const h = realHost(t, { engine: 'rust', uv: 'echo "usage: inbox" >&2; exit 2' });
  assert.deepEqual(await availability(h), { state: 'unavailable', reason: 'no-app' });
});

test('after a rollback to Python the inbox says unavailable', async t => {
  const h = realHost(t, { engine: 'python', uv: 'echo "usage: inbox" >&2; exit 2' });
  assert.deepEqual(await availability(h), { state: 'unavailable', reason: 'no-app' });
});

test('a real launcher that answers the probe is ready', async t => {
  const h = realHost(t, { engine: 'python', uv: 'echo "herdr-inbox app v1"' });
  assert.equal((await availability(h)).state, 'ready');
});
