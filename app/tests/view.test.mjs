// What the overlay says, read from what it draws, on every fixture feed.
import assert from 'node:assert/strict';
import { readdirSync } from 'node:fs';
import { register } from 'node:module';
import test from 'node:test';

register('./support/loader.mjs', import.meta.url);
const { render } = await import('../mods/herdr-inbox/view.js');
const { windowOf } = await import('../mods/herdr-inbox/view-text.js');
const { deliveryWords, retryLabel } = await import('../mods/herdr-inbox/words.js');
const { keyName, pastedKeys } = await import('../mods/herdr-inbox/keymap.js');
const { Rig, asker, failed, fixture, group, noOmissions, world } = await import('./support/rig.mjs');

const t = new Proxy({}, { get: (_, name) => String(name) });
const drawn = (rig, over = {}) => render({
  session: rig.session, status: { state: 'ready', root: '/fake/plugin' }, handle: {}, agentsNow: () => [], wordsNow: () => ({ title: 'Inbox', detail: 'Waiting' }), ...over,
}, t).texts.join('\n');

for (const file of readdirSync(new URL('../../tests/fixtures/app-feed/', import.meta.url))) {
  test(`draws ${file} in every row it can open`, () => {
    const r = new Rig(fixture(file.replace('.json', '')));
    assert.ok(drawn(r).length > 0);
    for (const entry of r.popup?.entries ?? []) {
      const s = new Rig(fixture(file.replace('.json', '')));
      s.popup.view.cursor = entry;
      if (s.popup.view.kind === 'list') s.press('enter');
      assert.ok(drawn(s).length > 0, entry);
    }
  });
}

test('the empty state shows only on a whole, good feed', () => {
  assert.match(drawn(new Rig(fixture('empty'))), /No open questions/);
  const left = drawn(new Rig(fixture('omitted-only')));
  assert.equal(left.includes('No open questions'), false);
  assert.match(left, /not empty/);
  assert.match(left, /2 questions/);
  assert.match(left, /Press o/);
  assert.match(left, /left out to stay under 1 MiB/);
  assert.equal(drawn(new Rig(fixture('not-migrated'))).includes('No open questions'), false);
});

test('partial: the counts and the errors stay on screen, cut text says so', () => {
  const r = new Rig(fixture('partial'));
  const list = drawn(r);
  assert.match(list, /1 undelivered answer/);
  assert.match(list, /remote machines could not be read/);
  assert.match(list, /1 remote row/);
});

test('a cut question offers the whole of it', () => {
  const cut = group(1, [1]);
  cut.questions[0].cut = true;
  const r = new Rig(world([cut]));
  assert.match(drawn(r), /cut to fit the feed/);
  assert.match(drawn(r), /Press f/);
});

test('ids past the cap are counted, with where to find them', () => {
  const r = new Rig(world([group(1), group(2)], { omitted: { ...noOmissions(), groups: 3, questions: 1500, question_ids: Array.from({ length: 1000 }, (_, i) => i + 100) } }));
  assert.match(drawn(r), /500 of those have no id here/);
});

test('a stale feed says so, and a stale row says what changed', () => {
  const r = new Rig(world([group(1), group(2)]));
  r.read(null);
  assert.match(drawn(r), /last good copy/);
});

test('the row under the cursor is always drawn', () => {
  for (let total = 1; total < 30; total += 1) {
    for (let at = 0; at < total; at += 1) {
      const { from, to } = windowOf(at, total, 4);
      assert.ok(from <= at && at < to && to - from <= 4 && from >= 0 && to <= total);
    }
  }
  const many = Array.from({ length: 9 }, (_, i) => ({ label: `Option ${i}`, description: null, preview: null }));
  const g = group(1, [1]);
  g.questions[0].options = many;
  const r = new Rig(world([g]));
  r.press('up'); // wraps to the last row
  assert.match(drawn(r), /Chat about this/);
});

test('a pending answer that retries by itself is not called failed', () => {
  assert.equal(deliveryWords(failed(1, { state: 'pending', wait: 'busy', auto_retry: true })).warn, false);
  assert.equal(deliveryWords(failed(1, { state: 'failed' })).warn, true);
  assert.match(deliveryWords(failed(1, { state: 'future_state', reason: 'Question Text 345' })).line, /Question Text 345/);
});

test('Send now is for a TUI row that waits for a busy agent, never a core row', () => {
  const busy = { state: 'pending', wait: 'busy' };
  assert.equal(retryLabel(failed(1, busy)), 'Send now');
  assert.equal(retryLabel(failed(1, { state: 'failed' })), 'Retry delivery');
  assert.equal(retryLabel(failed(1, busy, { asker: asker({ host: 'herdr-core', pane: null, terminal: 't3' }) })), 'Retry delivery');
});

test('host keys become the model\'s names', () => {
  const key = (k, m = {}, c) => keyName({ key: k, key_char: c, modifiers: m });
  assert.equal(key('tab', { shift: true }), 'shift+tab');
  assert.equal(key('enter'), 'enter');
  assert.equal(key('d', {}, 'd'), 'd');
  assert.equal(key('d', { shift: true }, 'D'), 'D');
  assert.equal(key('space', {}, ' '), 'space');
  assert.equal(key('a', { control: true }), 'ctrl+a');
  assert.equal(key('shift', { shift: true }), null);
  assert.deepEqual(pastedKeys('a b\nc'), ['a', 'space', 'b', 'space', 'c']);
});

test('the gone row counts the gone answers, not every ⚠ row', () => {
  const gone = id => failed(id, { state: 'agent_gone', reason: 'Question Text 345' });
  const r = new Rig(world([gone(1), gone(2), failed(3), group(9), group(10)]));
  assert.match(drawn(r), /2 answers whose agents are gone/);
});

test('a closed row keeps its words, and only a finished one a tick', () => {
  const r = new Rig(world([group(1), group(5)]), { standin: { host: 'gone' } });
  r.press('enter', 'enter');
  const text = drawn(r);
  assert.match(text, /Question Title 123 · recorded, nothing sent/);
  assert.equal(/✓/.test(text), false);
  assert.equal(/sending/.test(text), false);
});

test('a change under an open row is on screen before any send', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.press('enter');
  r.db.find('group:1').questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  r.published();
  assert.match(drawn(r), /changed since you opened it/);
});

test('Press f shows only where f works', () => {
  const feed = fixture('every-kind');
  const g = feed.entries.find(e => e.kind === 'remote_group');
  g.questions[0].cut = true;
  const r = new Rig(feed);
  r.session.popup.view.cursor = g.key;
  r.press('enter');
  assert.equal(r.popup.mode, 'remote');
  assert.equal(/Press f/.test(drawn(r)), false);
});

test('the pick hint counts only what digits reach', () => {
  const g = group(1, [1]);
  g.questions[0].multi_select = true;
  const r = new Rig(world([g]));
  assert.match(drawn(r), /1-3/); // two options and Something else; digits skip Submit
});
