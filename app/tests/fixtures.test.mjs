// The stand-in for B4's verbs follows the contract: every fixture case, exit and stdout, byte for byte.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { StandIn, tokenOf } from './support/standin.mjs';

const read = path => JSON.parse(readFileSync(new URL(`../../tests/fixtures/${path}`, import.meta.url)));
const { cases } = read('app-commands/commands.json');

const closes = (key, word) => s => s.remove(key, word);
const SETUP = {
  'stale-answered': closes('group:12', 'answered'),
  'stale-dismissed': closes('group:12', 'dismissed'),
  'stale-withdrawn': closes('group:12', 'withdrawn'),
  'stale-removed': closes('group:12', 'removed'),
  'stale-delivered': closes('failed:8', 'delivered'),
  'stale-fetched': closes('failed:8', 'fetched'),
  'stale-hidden': closes('failed:8', 'hidden'),
  'stale-acked': closes('alert:6', 'acked'),
  'stale-cleared': closes('alert:6', 'cleared'),
  'show-closed': closes('group:12', 'answered'),
  'retry-already-sending': s => s.claims.add(8),
  busy: s => { s.lock = 'busy'; },
  'not-migrated': s => { s.lock = 'not_migrated'; },
  error: s => { s.lock = 'error'; },
};

for (const c of cases) {
  test(`fixture: ${c.name}`, () => {
    const world = read(`app-feed/${c.world}`);
    const rows = c.stdout?.row ? [c.stdout.row] : [];
    const s = new StandIn(world, { server: c.server, host: c.host, session: c.session, rows });
    if (c.stdout?.row) s.full = new Map([[c.stdout.row.key, c.stdout.row]]);
    if (c.replay_of) s.run(cases.find(x => x.name === c.replay_of).argv.slice(1));
    SETUP[c.name]?.(s);
    const result = s.run(c.argv.slice(1).map(String));
    assert.equal(result.code, c.exit, result.stdout);
    if (c.stdout === null) assert.equal(result.stdout, '');
    else assert.deepEqual(JSON.parse(result.stdout), c.stdout);
  });
}

test('the stand-in computes the fixtures\' tokens', () => {
  for (const name of ['every-kind', 'partial', 'stale-token']) {
    for (const e of read(`app-feed/${name}.json`).entries) {
      if (e.token !== null && !e.questions?.some(q => q.cut) && !e.cut) assert.equal(tokenOf(e), e.token, e.key);
    }
  }
});
