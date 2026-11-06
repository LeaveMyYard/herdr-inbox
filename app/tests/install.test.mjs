import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
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

test('the record names the plugin, and says a copy can move and the operator\'s checkout cannot', t => {
  const copy = sandbox(t);
  ok(copy.run('install', { extra: { CLAUDE_CONFIG_DIR: '/fake/claude' } }));
  assert.equal(copy.receipt().plugin_root, copy.plugin);
  assert.equal(copy.receipt().plugin_moves, true);
  assert.equal(copy.receipt().claude_config, '/fake/claude');
  const checkout = sandbox(t, { checkout: true });
  ok(checkout.run('install'));
  assert.equal(checkout.receipt().plugin_moves, false);
  assert.equal(checkout.receipt().claude_config, null);
});

test('a herdr install is a git checkout that moves: each update is a new generation', t => {
  const s = sandbox(t, { checkout: true, root: 'config/herdr/plugins/github-installations/leavemyyard.herdr-inbox-1a2b/1259152-0/checkout' });
  ok(s.run('install'));
  assert.equal(s.receipt().plugin_moves, true);
});

test('a relative CLAUDE_CONFIG_DIR is not recorded: the record must stay readable', t => {
  const s = sandbox(t);
  ok(s.run('install', { extra: { CLAUDE_CONFIG_DIR: 'claude' } }));
  assert.equal(s.receipt().claude_config, null);
  ok(s.run('uninstall'));
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

for (const path of ['mods/herdr-inbox/main.js', 'data/herdr-inbox/notes.txt']) {
  test(`what is already at ${path} without a record is a collision`, t => {
    const s = sandbox(t);
    mkdirSync(s.inHome(join(path, '..')), { recursive: true });
    writeFileSync(s.inHome(path), 'Project Alpha\n');
    const before = tree(s.home);
    refused(s.run('install'), /Name collision/);
    assert.deepEqual(tree(s.home), before);
  });
}

for (const link of ['data', 'mods', 'data/herdr-inbox']) {
  test(`a link at ${link} is not followed`, t => {
    const s = sandbox(t);
    mkdirSync(join(s.temp, 'elsewhere'));
    mkdirSync(s.inHome('data'));
    if (link === 'data') rmSync(s.inHome('data'), { recursive: true });
    symlinkSync(join(s.temp, 'elsewhere'), s.inHome(link));
    assert.equal(s.run('install').status, 1);
    assert.deepEqual(tree(join(s.temp, 'elsewhere')), {});
  });
}

test('a link further down the record\'s own path is not followed: the walk refuses it', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  const moved = join(s.temp, 'elsewhere');
  renameSync(s.inHome('data/herdr-inbox'), moved);
  symlinkSync(moved, s.inHome('data/herdr-inbox'));
  const before = tree(moved);
  assert.equal(s.run('uninstall').status, 1);
  assert.deepEqual(tree(moved), before);
  assert.ok(existsSync(s.inHome('mods/herdr-inbox/main.js')));
});

test('a link in the package is refused', t => {
  const s = sandbox(t);
  symlinkSync('/fake/elsewhere', join(s.plugin, 'app/mods/herdr-inbox/linked.js'));
  refused(s.run('install'), /A package file is a link/);
  assert.deepEqual(tree(s.home), {});
});

test('a second installer in the same home is refused, and writes nothing', async t => {
  const s = sandbox(t);
  const code = `import fcntl, os, sys, time
fd = os.open(sys.argv[1], os.O_RDONLY)
fcntl.flock(fd, fcntl.LOCK_EX)
print('held', flush=True)
time.sleep(60)`;
  const child = spawn('python3', ['-c', code, s.home], { stdio: ['ignore', 'pipe', 'inherit'] });
  t.after(() => child.kill());
  await new Promise(resolve => child.stdout.once('data', resolve));
  refused(await new Promise(resolve => {
    const late = spawn('python3', [join(repo, 'scripts/app-install'), 'install', '--app-home', s.home, '--plugin-root', s.plugin, '--checker', s.checker], { env: s.env });
    let stderr = '';
    late.stderr.on('data', data => { stderr += data; });
    late.on('exit', status => resolve({ status, stderr, stdout: '' }));
  }), /Another install is running/);
  assert.deepEqual(tree(s.home), {});
});

for (const python of [[3, 10], [3, 9]]) {
  test(`Python ${python.join('.')} is refused before anything is written`, t => {
    const s = sandbox(t);
    const code = `import runpy, sys
sys.version_info = (${python.join(', ')}, 0, 'final', 0)
sys.argv = ['app-install', 'install', '--app-home', sys.argv[1], '--plugin-root', sys.argv[2], '--checker', sys.argv[3]]
runpy.run_path(${JSON.stringify(join(repo, 'scripts/app-install'))}, run_name='__main__')`;
    const result = spawnSync('python3', ['-c', code, s.home, s.plugin, s.checker], { env: s.env, encoding: 'utf8' });
    refused(result, /needs Python 3\.11 or later.*Nothing was changed/s);
    assert.deepEqual(tree(s.home), {});
  });
}

test('rename_new never replaces, and swap exchanges both whole', t => {
  const s = sandbox(t);
  const code = `import os, sys
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import atomic
fd = os.open(sys.argv[2], os.O_RDONLY)
os.mkdir('a', dir_fd=fd); os.mkdir('b', dir_fd=fd); os.mkdir('empty', dir_fd=fd)
open(sys.argv[2] + '/a/from-a', 'w').close(); open(sys.argv[2] + '/b/from-b', 'w').close()
try:
    atomic.rename_new(fd, 'a', fd, 'empty')  # a plain rename would replace an empty folder
    print('replaced')
except OSError as error:
    print('refused', error.errno != 0)
print(sorted(os.listdir(sys.argv[2] + '/b')))
atomic.swap(fd, 'a', fd, 'b')
print(sorted(os.listdir(sys.argv[2] + '/a')), sorted(os.listdir(sys.argv[2] + '/b')))`;
  const result = spawnSync('python3', ['-c', code, join(repo, 'app/installer'), s.home], { encoding: 'utf8' });
  assert.equal(result.stdout, "refused True\n['from-b']\n['from-b'] ['from-a']\n", result.stderr);
});

test('runtime files with no record are what a crash left: the install takes them over', t => {
  const s = sandbox(t);
  mkdirSync(s.inHome('data/herdr-inbox/stage/herdr-inbox'), { recursive: true });
  writeFileSync(s.inHome('data/herdr-inbox/control.json'), '{"v":1,"disabled":true}\n');
  ok(s.run('install'));
  assert.equal(readFileSync(s.inHome('data/herdr-inbox/control.json'), 'utf8'), '{"v":1,"disabled":false}\n');
  ok(s.run('uninstall'));
  assert.deepEqual(tree(s.home), {});
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
