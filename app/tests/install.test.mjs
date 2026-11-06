import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, readFileSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { lived, repo, sandbox, tree } from './support/world.mjs';

const ok = result => assert.equal(result.status, 0, result.stderr || result.stdout);
const refused = (result, pattern) => {
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stderr, pattern);
};

test('every shipped mod says place none, so nothing docks', () => {
  assert.equal(JSON.parse(readFileSync(join(repo, 'app/mods/herdr-inbox/mod.json'), 'utf8')).place, 'none');
});

test('an install puts in the mods and the runtime files, and touches nothing else', t => {
  const s = sandbox(t);
  lived(s);
  const before = tree(s.home);
  ok(s.run('install'));
  const after = tree(s.home);
  for (const path of Object.keys(before)) {
    if (path !== 'data/' && path !== 'mods/') assert.deepEqual(after[path], before[path], path);
  }
  const added = Object.keys(after).filter(path => !(path in before));
  assert.ok(added.every(path => /^(mods|data)\/herdr-inbox/.test(path)), added.join(' '));
  assert.equal(readFileSync(s.inHome('mods/herdr-inbox/main.js'), 'utf8'), readFileSync(join(s.plugin, 'app/mods/herdr-inbox/main.js'), 'utf8'));
});

test('the data folder and what holds question text or the record are private', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  const mode = path => statSync(s.inHome(path)).mode & 0o777;
  assert.equal(mode('data/herdr-inbox'), 0o700);
  assert.equal(mode('data/herdr-inbox/install.json'), 0o600);
  assert.equal(mode('data/herdr-inbox/control.json'), 0o600);
  assert.equal(readFileSync(s.inHome('data/herdr-inbox/control.json'), 'utf8'), '{"v":1,"disabled":false}\n');
  for (const lock of ['ensure.lock', 'writer.lock']) assert.equal(statSync(s.inHome(`data/herdr-inbox/${lock}`)).size, 0);
  assert.ok(!existsSync(s.inHome('data/herdr-inbox/stage')));
});

test('a data folder the app made first is taken over and made private', t => {
  const s = sandbox(t);
  mkdirSync(s.inHome('data/herdr-inbox'), { recursive: true, mode: 0o755 });
  ok(s.run('install'));
  assert.equal(statSync(s.inHome('data/herdr-inbox')).mode & 0o777, 0o700);
});

test('the record names the plugin, and says a copy can move and a checkout cannot', t => {
  const copy = sandbox(t);
  ok(copy.run('install', { extra: { CLAUDE_CONFIG_DIR: '/fake/claude' } }));
  assert.equal(copy.receipt().plugin_root, copy.plugin);
  assert.equal(copy.receipt().plugin_copy, true);
  assert.equal(copy.receipt().claude_config, '/fake/claude');
  const checkout = sandbox(t, { checkout: true });
  ok(checkout.run('install'));
  assert.equal(checkout.receipt().plugin_copy, false);
  assert.equal(checkout.receipt().claude_config, null);
});

test('a folder that is not the inbox plugin is refused', t => {
  const s = sandbox(t);
  writeFileSync(join(s.plugin, 'herdr-plugin.toml'), 'id = "leavemyyard.herdr-panorama"\n');
  const before = tree(s.home);
  refused(s.run('install'), /Not the inbox plugin/);
  assert.deepEqual(tree(s.home), before);
});

test('a mod that would dock is refused', t => {
  const s = sandbox(t);
  writeFileSync(join(s.plugin, 'app/mods/herdr-inbox/mod.json'), '{ "place": "right" }\n');
  refused(s.run('install'), /must declare place none/);
  assert.deepEqual(tree(s.home), {});
});

test('a mod folder of another name is refused: only herdr-inbox names are ours', t => {
  const s = sandbox(t);
  mkdirSync(join(s.plugin, 'app/mods/panorama'));
  refused(s.run('install'), /Not a herdr-inbox mod folder: panorama/);
  assert.deepEqual(tree(s.home), {});
});

test('a check that fails installs nothing', t => {
  const s = sandbox(t);
  lived(s);
  writeFileSync(`${s.checker}.fail`, '');
  const before = tree(s.home);
  refused(s.run('install'), /herdr-app check failed for herdr-inbox: Question Title 123/);
  assert.deepEqual(tree(s.home), before);
});

for (const path of ['mods/herdr-inbox/main.js', 'data/herdr-inbox/feed.json']) {
  test(`what is already at ${path} without a record is a collision`, t => {
    const s = sandbox(t);
    mkdirSync(s.inHome(join(path, '..')), { recursive: true });
    writeFileSync(s.inHome(path), 'Project Alpha\n');
    const before = tree(s.home);
    refused(s.run('install'), /Name collision/);
    assert.deepEqual(tree(s.home), before);
  });
}

test('a link in the home is not followed', t => {
  const s = sandbox(t);
  mkdirSync(join(s.temp, 'elsewhere'));
  symlinkSync(join(s.temp, 'elsewhere'), s.inHome('data'));
  assert.equal(s.run('install').status, 1);
  assert.deepEqual(tree(join(s.temp, 'elsewhere')), {});
});

test('an app home reached through a link is the folder it names', t => {
  const s = sandbox(t);
  const link = join(s.temp, 'home-link');
  symlinkSync(s.home, link);
  const result = spawnSync('python3', [join(repo, 'scripts/app-install'), 'install', '--app-home', link,
    '--plugin-root', s.plugin, '--checker', s.checker], { env: s.env, encoding: 'utf8' });
  ok(result);
  assert.ok(existsSync(s.inHome('mods/herdr-inbox/main.js')));
});

test('an app home that is not an absolute path is refused', t => {
  const s = sandbox(t);
  const result = spawnSync('python3', [join(repo, 'scripts/app-install'), 'uninstall', '--app-home', 'home'],
    { cwd: s.temp, env: s.env, encoding: 'utf8' });
  refused(result, /must be absolute/);
});

test('an install that fails half way leaves nothing', t => {
  const s = sandbox(t);
  mkdirSync(s.inHome('mods'));
  chmodSync(s.inHome('mods'), 0o555); // the stage, record and runtime files go in; the mods cannot
  const before = tree(s.home);
  assert.equal(s.run('install').status, 1);
  assert.deepEqual(tree(s.home), before);
});
