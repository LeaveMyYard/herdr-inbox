import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { lived, repo, sandbox, tree } from './support/world.mjs';

const ok = result => assert.equal(result.status, 0, result.stderr || result.stdout);

// Holds flock on a file in a process of its own, as the writer or `--ensure` would.
async function holder(t, path) {
  const code = `import fcntl, sys, time
fd = open(sys.argv[1], 'r+')
fcntl.flock(fd, fcntl.LOCK_EX)
print('held', flush=True)
time.sleep(60)`;
  const child = spawn('python3', ['-c', code, path], { stdio: ['ignore', 'pipe', 'inherit'] });
  t.after(() => child.kill());
  await new Promise(resolve => child.stdout.once('data', resolve));
  return child;
}

function running(s) {
  for (const [name, text] of [['feed.json', '{"v":1}\n'], ['.feed.tmp', '{'], ['writer.json', '{"v":1}\n'],
    ['stop.request', ''], ['start.json', '{"v":1}\n']]) {
    writeFileSync(s.inHome(`data/herdr-inbox/${name}`), text, { mode: 0o600 });
  }
  for (const name of ['gpui-kit.d.ts', 'jsconfig.json']) writeFileSync(s.inHome(`mods/herdr-inbox/${name}`), '// host\n');
}

test('in an empty home, uninstall leaves nothing at all', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  ok(s.run('uninstall'));
  assert.deepEqual(tree(s.home), {});
});

test('in a home in use, uninstall leaves it byte for byte as it was, Panorama included', t => {
  const s = sandbox(t);
  lived(s);
  const before = tree(s.home);
  ok(s.run('install'));
  running(s);
  ok(s.run('uninstall'));
  assert.deepEqual(tree(s.home), before);
});

test('uninstall needs no package: the plugin may be gone or be another version', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  rmSync(s.plugin, { recursive: true });
  ok(s.run('uninstall'));
  assert.deepEqual(tree(s.home), {});
});

test('uninstall keeps what is not ours, says so, and still removes the feed', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  running(s);
  writeFileSync(s.inHome('mods/herdr-inbox/main.js'), '// mine\n');
  writeFileSync(s.inHome('data/herdr-inbox/notes.txt'), 'Project Alpha\n');
  const result = s.run('uninstall');
  ok(result);
  assert.match(result.stdout, /Kept, not ours: mods\/herdr-inbox\/main\.js/);
  assert.match(result.stdout, /Kept, not ours: data\/herdr-inbox\/notes\.txt/);
  assert.deepEqual(Object.keys(tree(s.home)).sort(), ['data/', 'data/herdr-inbox/', 'data/herdr-inbox/notes.txt', 'mods/', 'mods/herdr-inbox/', 'mods/herdr-inbox/main.js']);
});

test('with nothing installed, uninstall refuses and removes nothing', t => {
  const s = sandbox(t);
  lived(s);
  const before = tree(s.home);
  assert.equal(s.run('uninstall').status, 1);
  assert.deepEqual(tree(s.home), before);
});

const WINDOW = '{"layout":"main","errors":[]}';
const USES = '{ "content": { "mod": "herdr-inbox" } }\n';

function stopped(s, pattern) {
  const before = tree(s.home);
  const result = s.run('uninstall');
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stderr, pattern);
  assert.deepEqual(tree(s.home), before);
}

function shown(t) {
  const s = sandbox(t);
  lived(s);
  ok(s.run('install'));
  s.window(WINDOW);
  return s;
}

test('a layout on screen that shows the mod stops the uninstall', t => {
  const s = shown(t);
  writeFileSync(s.inHome('layouts/main.json'), USES);
  stopped(s, /Layout main is showing herdr-inbox/);
  assert.equal(readFileSync(join(s.tools, 'status.log'), 'utf8').trim().split('\n').at(-1), s.home);
});

test('what shows is what the window says, not what shared.json says', t => {
  const s = shown(t);
  writeFileSync(s.inHome('layouts/project-alpha.json'), USES);
  s.window('{"layout":"project-alpha"}');
  stopped(s, /Layout project-alpha is showing herdr-inbox/);
});

test('a layout on screen reached through a link is read', t => {
  const s = shown(t);
  writeFileSync(join(s.temp, 'kept.json'), USES);
  rmSync(s.inHome('layouts/main.json'));
  symlinkSync(join(s.temp, 'kept.json'), s.inHome('layouts/main.json'));
  stopped(s, /Layout main is showing herdr-inbox/);
});

test('a layouts folder that is a link is read', t => {
  const s = shown(t);
  mkdirSync(join(s.temp, 'dotfiles'));
  writeFileSync(join(s.temp, 'dotfiles/main.json'), USES);
  rmSync(s.inHome('layouts'), { recursive: true });
  symlinkSync(join(s.temp, 'dotfiles'), s.inHome('layouts'));
  stopped(s, /Layout main is showing herdr-inbox/);
});

test('a shown layout that does not parse stops the uninstall: the host keeps its last good version', t => {
  const s = shown(t);
  writeFileSync(s.inHome('layouts/main.json'), '{ broken');
  stopped(s, /Layout main is on screen and its file cannot be read/);
});

test('a shown layout whose file was renamed stops the uninstall: the host keeps showing it', t => {
  const s = shown(t);
  rmSync(s.inHome('layouts/main.json'));
  stopped(s, /Layout main is on screen and its file cannot be read/);
});

for (const [name, out, code] of [['nothing', '', 0], ['no JSON', 'nonsense', 0], ['no layout', '{"zoom":1}', 0],
  ['an empty layout', '{"layout":""}', 0], ['a layout that is a path', '{"layout":"../main"}', 0], ['a failure', '', 1],
  ['a crash', '', 139]]) {
  test(`a window that answers with ${name} stops the uninstall`, t => {
    const s = shown(t);
    s.window(out, code);
    stopped(s, /did not say what it shows/);
  });
}

test('a window that does not answer in time stops the uninstall', t => {
  const s = shown(t);
  writeFileSync(join(s.tools, 'status.delay'), '6');
  stopped(s, /did not say what it shows/);
});

test('with no window, nothing shows the mod: it goes, and the layout is named', t => {
  const s = sandbox(t);
  lived(s);
  ok(s.run('install'));
  writeFileSync(s.inHome('layouts/main.json'), USES);
  const result = s.run('uninstall');
  ok(result);
  assert.match(result.stdout, /Layout main still names a removed mod/);
  assert.ok(!existsSync(s.inHome('mods/herdr-inbox')));
});

test('with no herdr-app at all there is no window to ask, and uninstall still works', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  const python = spawnSync('python3', ['-c', 'import sys; print(sys.executable)'], { encoding: 'utf8' }).stdout.trim();
  const result = spawnSync(python, [join(repo, 'scripts/app-install'), 'uninstall', '--app-home', s.home], { env: { PATH: '/nonexistent' }, encoding: 'utf8' });
  ok(result);
  assert.deepEqual(tree(s.home), {});
});

test('a layout not on screen is only named', t => {
  const s = shown(t);
  writeFileSync(s.inHome('layouts/project-alpha.json'), USES);
  const result = s.run('uninstall');
  ok(result);
  assert.match(result.stdout, /Layout project-alpha still names a removed mod/);
  assert.ok(existsSync(s.inHome('layouts/project-alpha.json')));
});

test('a host file name further down is an added file, not the host\'s', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  mkdirSync(s.inHome('mods/herdr-inbox/notes'));
  writeFileSync(s.inHome('mods/herdr-inbox/notes/jsconfig.json'), 'Project Alpha\n');
  const result = s.run('uninstall');
  ok(result);
  assert.match(result.stdout, /Kept, not ours: mods\/herdr-inbox\/notes\/jsconfig\.json/);
});

test('uninstall reports a link at the mod\'s path and leaves it', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  rmSync(s.inHome('mods/herdr-inbox'), { recursive: true });
  symlinkSync(s.plugin, s.inHome('mods/herdr-inbox'));
  const result = s.run('uninstall');
  ok(result);
  assert.match(result.stdout, /Kept, not ours: mods\/herdr-inbox/);
});

test('uninstall reports an empty folder someone added', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  mkdirSync(s.inHome('mods/herdr-inbox/notes'));
  const result = s.run('uninstall');
  ok(result);
  assert.match(result.stdout, /Kept, not ours: mods\/herdr-inbox\/notes/);
});

test('with no record, a folder of runtime files is cleared; one with anything else is refused', t => {
  const s = sandbox(t);
  mkdirSync(s.inHome('data/herdr-inbox/stage'), { recursive: true });
  writeFileSync(s.inHome('data/herdr-inbox/feed.json'), '{"v":1}\n');
  ok(s.run('uninstall'));
  assert.deepEqual(tree(s.home), {});
  mkdirSync(s.inHome('data/herdr-inbox'), { recursive: true });
  writeFileSync(s.inHome('data/herdr-inbox/notes.txt'), 'Project Alpha\n');
  stopped(s, /Nothing is installed here/);
});

test('with no record, a mod folder is not ours to touch', t => {
  const s = sandbox(t);
  mkdirSync(s.inHome('mods/herdr-inbox'), { recursive: true });
  mkdirSync(s.inHome('data/herdr-inbox'), { recursive: true });
  stopped(s, /Nothing is installed here/);
});

test('a writer that stays refuses the uninstall and stays disabled; it can be run again', async t => {
  const s = sandbox(t);
  ok(s.run('install'));
  const writer = await holder(t, s.inHome('data/herdr-inbox/writer.lock'));
  const before = tree(s.home);
  const start = Date.now();
  const result = s.run('uninstall');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /did not stop in 2 s/);
  assert.ok(Date.now() - start >= 2000);
  assert.equal(readFileSync(s.inHome('data/herdr-inbox/control.json'), 'utf8'), '{"v":1,"disabled":true}\n');
  const { 'data/herdr-inbox/control.json': _control, ...rest } = tree(s.home);
  const { 'data/herdr-inbox/control.json': _old, ...expected } = before;
  assert.deepEqual(rest, expected);
  writer.kill();
  await new Promise(resolve => writer.once('exit', resolve));
  ok(s.run('uninstall'));
  assert.deepEqual(tree(s.home), {});
});

test('uninstall waits for a writer start in progress, and refuses past its limit', async t => {
  const s = sandbox(t);
  ok(s.run('install'));
  await holder(t, s.inHome('data/herdr-inbox/ensure.lock'));
  const before = tree(s.home);
  const result = s.run('uninstall');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /A writer start did not finish/);
  assert.deepEqual(tree(s.home), before);
});

for (const [name, path] of [['outside the home', 'mods/herdr-inbox/../../secret'], ["another mod's", 'mods/panorama/main.js'],
  ['an absolute', '/fake/main.js']]) {
  test(`a record that names ${name} file is refused before anything goes`, t => {
    const s = sandbox(t);
    lived(s);
    ok(s.run('install'));
    const record = s.receipt();
    record.files[path] = 'a'.repeat(64);
    writeFileSync(s.inHome('data/herdr-inbox/install.json'), JSON.stringify(record), { mode: 0o600 });
    const before = tree(s.home);
    const result = s.run('uninstall');
    assert.equal(result.status, 1);
    assert.match(result.stderr, /not ours/);
    assert.deepEqual(tree(s.home), before);
  });
}

test('a record that names a folder of the operator\'s is refused', t => {
  const s = sandbox(t);
  lived(s);
  ok(s.run('install'));
  writeFileSync(s.inHome('data/herdr-inbox/install.json'), JSON.stringify({ ...s.receipt(), dirs: ['layouts'] }), { mode: 0o600 });
  const before = tree(s.home);
  assert.equal(s.run('uninstall').status, 1);
  assert.deepEqual(tree(s.home), before);
});

test('a record anyone can read is refused', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  const path = s.inHome('data/herdr-inbox/install.json');
  const text = readFileSync(path);
  rmSync(path);
  writeFileSync(path, text, { mode: 0o644 });
  assert.equal(s.run('uninstall').status, 1);
  assert.ok(existsSync(s.inHome('mods/herdr-inbox/main.js')));
});
