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

// What was drawn, with the handlers the mod's clicks go to: they go through the session as main.js's do.
const tree = rig => render({
  session: rig.session, status: { state: 'ready', root: '/fake/plugin' }, handle: {}, agentsNow: () => [], wordsNow: () => ({ title: 'Inbox', detail: 'Waiting' }),
  onRow: row => { rig.tick(); rig.session.click(row, rig.now); rig.pump(); },
  onTab: tab => { rig.tick(); rig.session.clickTab(tab, rig.now); rig.pump(); },
  onLeftOut: id => { rig.tick(); rig.session.clickLeftOut(id); rig.pump(); },
  onButton: name => { rig.tick(); rig.session.button(name, rig.now); rig.pump(); },
}, t);
const styled = (el, ...call) => el.calls.some(c => JSON.stringify(c) === JSON.stringify(call));
const clickOn = (rig, id) => {
  const el = tree(rig).ids.get(id);
  assert.ok(el?.click, `${id} is drawn and clickable`);
  el.click(null, {});
};

test('the Submit tab is set apart: solid, ticked, with how many are answered', () => {
  const r = new Rig(world([group(1, [1, 2, 3]), group(5)]));
  r.press('enter');
  const chip = () => tree(r).ids.get('tab-submit');
  assert.match(chip().texts.join(' '), /Submit ✓ 0\/3 answered/);
  assert.equal(styled(chip(), 'bg', 'solid'), false, 'nothing answered: not yet the solid one');
  r.press('enter'); // Q1 drafted
  assert.match(chip().texts.join(' '), /1\/3 answered/);
  assert.ok(styled(chip(), 'bg', 'solid'));
  assert.ok(styled(chip(), 'cursor_pointer'));
  const q = tree(r).ids.get('tab-q:1');
  assert.equal(styled(q, 'bg', 'solid'), false, 'a question tab is no button');
  assert.equal(q.texts.join(' ').includes('Submit'), false);
});

test('clicking the drawn tabs switches them, and the Submit tab opens the review', () => {
  const r = new Rig(world([group(1, [1, 2, 3]), group(5)]));
  r.press('enter');
  clickOn(r, 'tab-q:3');
  assert.equal(r.popup.current, 'q:3');
  clickOn(r, 'tab-submit');
  assert.equal(r.popup.view.kind, 'review');
});

test('the review draws Submit answers as the primary button and Cancel as the secondary one', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.press('enter', 'enter'); // Q1 drafted, on to Q2
  clickOn(r, 'tab-submit');
  const ids = tree(r).ids;
  const submit = ids.get('submit');
  const cancel = ids.get('cancel');
  assert.match(submit.texts.join(' '), /Submit answers ✓ Sends 1 of 2/);
  assert.ok(styled(submit, 'bg', 'solid') && styled(submit, 'text_color', 'solid_foreground'));
  assert.ok(styled(cancel, 'bg', 'raised'));
  assert.equal(styled(ids.get('item-1'), 'bg', 'solid'), false, 'an answer is no button');
  assert.ok(ids.get('review-actions').texts.includes('Cancel'), 'the actions sit apart from the answers');
});

test('clicking a drawn review item goes to it, and Submit answers sends what Enter sends', () => {
  const open = () => {
    const r = new Rig(world([group(1, [1, 2]), group(5)]));
    r.press('enter', 'enter');
    clickOn(r, 'tab-submit');
    r.tick();
    return r;
  };
  const item = open();
  clickOn(item, 'item-2');
  assert.equal(item.popup.current, 'q:2');
  const keyed = open();
  keyed.press('enter');
  const clicked = open();
  clickOn(clicked, 'submit');
  assert.ok(keyed.ran.some(a => a[1] === 'answer'), 'Enter sent the answers');
  assert.deepEqual(clicked.ran, keyed.ran);
});

test('list rows, an undelivered answer\'s choices and left-out rows are drawn clickable', () => {
  const r = new Rig(world([group(1), failed(8), group(9)], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] } }));
  for (const id of ['row-group:1', 'row-failed:8', 'row-group:9']) assert.ok(tree(r).ids.get(id)?.click, id);
  clickOn(r, 'row-failed:8');
  assert.equal(r.popup.mode, 'failed');
  for (const id of ['retry', 'hide']) assert.ok(tree(r).ids.get(id)?.click, id);
  r.press('escape', 'o');
  assert.ok(tree(r).ids.get('left-21')?.click);
});

test('a long question keeps whole lines and leaves the options a place to scroll', () => {
  const g = group(1, [1]);
  g.questions[0].question = Array.from({ length: 40 }, (_, i) => `Question line ${i + 1} of Question Title 123`).join('\n');
  g.questions[0].context = 'Context Text 456';
  g.questions[0].options = Array.from({ length: 5 }, (_, i) => ({ label: `Option ${i + 1}`, description: null, preview: null }));
  const r = new Rig(world([g]));
  const root = render({ session: r.session, status: { state: 'ready', root: '/fake/plugin' }, handle: {}, agentsNow: () => [], wordsNow: () => ({}), size: { width: 800, height: 600 } }, t);
  const call = (el, name) => el.calls.find(c => c[0] === name);
  const text = root.ids.get('q-text');
  assert.ok(text, 'the question text is its own scroll region');
  const cap = Number.parseInt(call(text, 'max_h')[1], 10);
  assert.equal(cap % 22, 0, 'the cap falls on a line boundary');
  assert.equal(call(text, 'gap'), undefined, 'a gap between question and context would take the cap off the line grid');
  assert.equal(text.texts.length, 2, 'question and context both live in the region');
  assert.ok(cap >= 3 * 22 && cap <= 600 / 3 + 22, 'a readable share of the window');
  assert.ok(call(text, 'overflow_y_scrollbar'), 'the rest is reachable by scrolling');
  assert.equal(call(root.ids.get('q'), 'min_h'), undefined, 'the text is not squeezed to a fixed small box');
  assert.ok(call(root.ids.get('q'), 'flex_none'), 'the text does not give its height to the options');
  const choices = root.ids.get('choices');
  assert.ok(call(choices, 'min_h') && call(choices, 'overflow_y_scrollbar'), 'the options are what scroll when the window runs out');
});

test('at any window height the question and the options it leaves room for fit together', () => {
  for (const height of [440, 520, 600, 700, 900]) {
    const g = group(1, [1]);
    g.questions[0].question = Array.from({ length: 40 }, (_, i) => `Question line ${i + 1} of Question Title 123`).join('\n');
    g.questions[0].context = 'Context Text 456';
    g.questions[0].options = Array.from({ length: 5 }, (_, i) => ({ label: `Option ${i + 1}`, description: `Detail ${i + 1}`, preview: null }));
    const r = new Rig(world([g]));
    const root = render({ session: r.session, status: { state: 'ready', root: '/fake/plugin' }, handle: {}, agentsNow: () => [], wordsNow: () => ({}), size: { width: 800, height: height } }, t);
    const cap = Number.parseInt(root.ids.get('q-text').calls.find(c => c[0] === 'max_h')[1], 10);
    const rows = [...root.ids.keys()].filter(id => /^(opt-|other|chat|next|submit)/.test(id)).length;
    assert.ok(rows >= 2, `${height}: at least two options are drawn`);
    assert.ok(300 + cap + 30 + Math.min(rows, 2) * 64 <= Math.max(height, 300 + 66 + 30 + 128), `${height}: two rows and the hint fit under the capped question`);
    if (height >= 700) assert.ok(rows >= 3, `${height}: a roomy window shows more than two options`);
  }
});
