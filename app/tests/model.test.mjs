// The overlay's model replays the popup transition golden: every scenario's events, at their times,
// give the same effects and the same state, step by step.
import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./support/loader.mjs', import.meta.url);
const { deviations, golden, played, withDeviation } = await import('./support/golden.mjs');
const data = await import('../mods/herdr-inbox/model/data.js');

const plain = value => JSON.parse(JSON.stringify(value));

for (const scenario of golden.scenarios) {
  test(`golden: ${scenario.name}`, () => {
    const wanted = withDeviation(scenario);
    played(wanted).forEach(([effects, state], i) => {
      const step = wanted.steps[i];
      const name = `step ${i} (${JSON.stringify(step.event).slice(0, 60)} at ${step.at})`;
      assert.deepEqual(plain(effects), step.effects, `${name}: effects`);
      assert.deepEqual(state, step.state, `${name}: state`);
    });
  });
}

test('the delays are the golden\'s', () => {
  assert.deepEqual(
    [data.VIEW_ARM, data.ROW_ARM, data.GRACE, data.RELEASE, data.GO_PAUSE],
    ['arm', 'row_arm', 'grace', 'release', 'go'].map(k => golden.delays_ms[k]),
  );
});

test('the golden is whole: every scenario has steps, and every deviation names a scenario', () => {
  assert.equal(golden.schema, 1);
  assert.equal(golden.scenarios.length, 149);
  for (const d of deviations.transitions) assert.ok(golden.scenarios.some(s => s.name === d.scenario), d.scenario);
});
