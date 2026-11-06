import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
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

test('a check that starts a server fails, says where, and nothing runs hc or signals it', t => {
  const s = sandbox(t);
  writeFileSync(`${s.checker}.socket`, '');
  const result = appCheck(s);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /A check started a server/);
  const socket = result.stderr.match(/(\/tmp\/hi-[^/]+)\/\S+\.sock/);
  assert.ok(socket, result.stderr);
  t.after(() => rmSync(socket[1], { recursive: true, force: true }));
  assert.ok(!existsSync(join(s.tools, 'hc.log')), 'hc ran');
});

test('no installer file runs hc, herdr-core or a signal: it cannot reach any core', () => {
  for (const name of readdirSync(join(repo, 'app/installer'))) {
    const text = readFileSync(join(repo, 'app/installer', name), 'utf8');
    const code = text.replace(/"""[\s\S]*?"""/g, '').replace(/#.*$/gm, '');
    assert.doesNotMatch(code, /['"](hc|herdr-core)['"]|os\.kill|\.kill\(|\.terminate\(|signal/, name);
  }
});

test('the check\'s core socket can only be in its private folder', () => {
  const code = `import sys
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import check
env = check.private_env('/tmp/hi-fake')
print(env['XDG_RUNTIME_DIR'], env['HOME'], env['HC_SESSION'].startswith('inbox-check-'))
try:
    check.private_env('/run/user/1000/fake')
    print('allowed')
except ValueError as error:
    print('refused')`;
  const result = spawnSync('python3', ['-c', code, join(repo, 'app/installer')], { encoding: 'utf8' });
  assert.equal(result.stdout, '/tmp/hi-fake/runtime /tmp/hi-fake True\nrefused\n', result.stderr);
});
