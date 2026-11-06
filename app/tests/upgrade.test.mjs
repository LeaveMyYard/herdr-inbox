import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { lived, sandbox, tree } from './support/world.mjs';

const ok = result => assert.equal(result.status, 0, result.stderr || result.stdout);

test('installing again replaces the mods, drops what left the package, and keeps the feed', t => {
  const s = sandbox(t);
  lived(s);
  writeFileSync(join(s.plugin, 'app/mods/herdr-inbox/old.js'), '// old\n');
  ok(s.run('install'));
  writeFileSync(s.inHome('data/herdr-inbox/feed.json'), '{"v":1}\n');
  writeFileSync(s.inHome('mods/herdr-inbox/gpui-kit.d.ts'), '// from the host\n');
  rmSync(join(s.plugin, 'app/mods/herdr-inbox/old.js'));
  writeFileSync(join(s.plugin, 'app/mods/herdr-inbox/new.js'), '// new\n');
  const result = s.run('install');
  ok(result);
  assert.match(result.stdout, /Upgraded/);
  assert.ok(!existsSync(s.inHome('mods/herdr-inbox/old.js')));
  assert.equal(readFileSync(s.inHome('mods/herdr-inbox/new.js'), 'utf8'), '// new\n');
  assert.equal(readFileSync(s.inHome('data/herdr-inbox/feed.json'), 'utf8'), '{"v":1}\n');
  assert.deepEqual(s.receipt().previous, {});
  assert.ok(!('mods/herdr-inbox/old.js' in s.receipt().files));
  assert.ok(!existsSync(s.inHome('data/herdr-inbox/stage')));
});

test('an upgrade from another root records the new root', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  const moved = join(s.temp, 'moved');
  ok(s.run('install', { root: (() => { mkdirSync(moved); return copied(s, moved); })() }));
  assert.equal(s.receipt().plugin_root, moved);
});

function copied(s, target) {
  for (const path of ['herdr-plugin.toml', 'scripts/launch', 'app/mods/herdr-inbox/main.js', 'app/mods/herdr-inbox/mod.json', 'app/mods/herdr-inbox/plugin.js']) {
    mkdirSync(join(target, path, '..'), { recursive: true });
    writeFileSync(join(target, path), readFileSync(join(s.plugin, path)));
  }
  return target;
}

test('a reinstall is the way out of disabled, and of the restart ledger', t => {
  const s = sandbox(t);
  ok(s.run('install'));
  writeFileSync(s.inHome('data/herdr-inbox/control.json'), '{"v":1,"disabled":true}\n', { mode: 0o600 });
  writeFileSync(s.inHome('data/herdr-inbox/start.json'), '{"v":1,"failures":8,"retry_at_ms":0}\n', { mode: 0o600 });
  ok(s.run('install'));
  assert.equal(readFileSync(s.inHome('data/herdr-inbox/control.json'), 'utf8'), '{"v":1,"disabled":false}\n');
  assert.ok(!existsSync(s.inHome('data/herdr-inbox/start.json')));
});

test('an upgrade refuses to lose an edit or an added file, and changes nothing', t => {
  for (const [path, text] of [['mods/herdr-inbox/main.js', '// mine\n'], ['mods/herdr-inbox/notes.txt', 'Project Alpha\n']]) {
    const s = sandbox(t);
    ok(s.run('install'));
    writeFileSync(s.inHome(path), text);
    const before = tree(s.home);
    const result = s.run('install');
    assert.equal(result.status, 1);
    assert.match(result.stderr, /Edited or added files would be lost/);
    assert.ok(result.stderr.includes(path));
    assert.deepEqual(tree(s.home), before);
  }
});

test('an upgrade that stopped half way can be run again', t => {
  const s = sandbox(t);
  writeFileSync(join(s.plugin, 'app/mods/herdr-inbox/old.js'), '// old\n');
  ok(s.run('install'));
  const finished = s.receipt();
  // what a crash after the journal leaves: the old tree in place, a new record, a stage
  rmSync(join(s.plugin, 'app/mods/herdr-inbox/old.js'));
  const previous = Object.fromEntries(Object.entries(finished.files).map(([path, sha256]) => [path, [sha256]]));
  const { 'mods/herdr-inbox/old.js': _gone, ...files } = finished.files;
  writeFileSync(s.inHome('data/herdr-inbox/install.json'), JSON.stringify({ ...finished, files, previous }), { mode: 0o600 });
  mkdirSync(s.inHome('data/herdr-inbox/stage/herdr-inbox'), { recursive: true });
  ok(s.run('install'));
  assert.ok(!existsSync(s.inHome('mods/herdr-inbox/old.js')));
  assert.deepEqual(s.receipt().previous, {});
});

test('a mod that left the package is removed, with the folder the app made for it', t => {
  const s = sandbox(t);
  const toast = join(s.plugin, 'app/mods/herdr-inbox-toast');
  mkdirSync(toast);
  writeFileSync(join(toast, 'main.js'), '// toast\n');
  writeFileSync(join(toast, 'mod.json'), '{ "place": "none" }\n');
  ok(s.run('install'));
  mkdirSync(s.inHome('data/herdr-inbox-toast'));
  rmSync(toast, { recursive: true });
  ok(s.run('install'));
  assert.ok(!existsSync(s.inHome('mods/herdr-inbox-toast')));
  assert.ok(!existsSync(s.inHome('data/herdr-inbox-toast')));
});
