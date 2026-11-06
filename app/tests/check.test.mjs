import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { repo, sandbox } from './support/world.mjs';

const appCheck = (s, checker = s.checker) => spawnSync('python3', [join(repo, 'scripts/app-check'), '--checker', checker],
  { env: s.env, encoding: 'utf8' });

test('app-check runs the checker on every mod, in a private home that is removed after', t => {
  const s = sandbox(t);
  const result = appCheck(s);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Checked herdr-inbox\./);
  const [checked] = readFileSync(`${s.checker}.log`, 'utf8').trim().split('\n');
  assert.ok(checked.startsWith('/tmp/hi-'), checked);
  assert.throws(() => readFileSync(join(checked, 'main.js')));
});

test('app-check fails when a mod does, with the check\'s own words', t => {
  const s = sandbox(t);
  writeFileSync(`${s.checker}.fail`, '');
  const result = appCheck(s);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Question Title 123/);
});

test('without herdr-app there is nothing to check with, and it says so', t => {
  const s = sandbox(t);
  const result = appCheck(s, 'herdr-app');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /herdr-app is required/);
});
