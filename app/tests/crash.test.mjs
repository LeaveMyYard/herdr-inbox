import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { repo, sandbox } from './support/world.mjs';

const python = spawnSync('python3', ['-c', 'import sys; print(sys.executable)'], { encoding: 'utf8' }).stdout.trim();

test('a crash at any disk call of an install, upgrade or uninstall can be run again from', t => {
  const a = sandbox(t);
  const b = sandbox(t, { root: 'plugin-b' });
  // an upgrade that adds a file and drops one, so that both directions are crashed through
  writeFileSync(join(a.plugin, 'app/mods/herdr-inbox/old.js'), '// old\n');
  writeFileSync(join(b.plugin, 'app/mods/herdr-inbox/new.js'), '// new\n');
  const result = spawnSync(python, [join(repo, 'app/tests/support/crash.py'), repo, a.plugin, b.plugin, a.temp],
    { env: { PATH: '/nonexistent' }, encoding: 'utf8', timeout: 240000 });
  assert.equal(result.status, 0, result.stderr);
  const { points, failures } = JSON.parse(result.stdout);
  assert.deepEqual(failures, []);
  for (const [name, count] of Object.entries(points)) assert.ok(count >= 15, `${name}: ${count} points`);
});
