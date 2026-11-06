// The host's side of the overlay: main.js itself, with keys, clicks and Esc as a window gives them.
import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./support/loader.mjs', import.meta.url);
const { default: HerdrInbox } = await import('../mods/herdr-inbox/main.js');
const { Window } = await import('./support/host.mjs');
const { asker, failed, group, noOmissions, world } = await import('./support/rig.mjs');

const choice = args => JSON.parse(args[args.indexOf('--detail') + 1]);
const open = (feed, options) => new Window(HerdrInbox, feed, options).opened();
const three = (id, over) => {
  const g = group(id, [id], over);
  g.questions[0].options = ['Option A', 'Option B', 'Option C'].map(label => ({ label, description: null, preview: null }));
  return g;
};

test('Esc is bound deeper than the host\'s, and the root takes it', async () => {
  const w = await open(world([group(1), group(2)]));
  assert.deepEqual(w.cx.bind_keys && w.bound, [{ keystroke: 'escape', action: 'herdr_inbox_escape', context: 'HerdrInbox' }]);
});

test('held Enter on a list opens nothing more and sends nothing', async () => {
  const w = await open(world([group(1), group(5)]));
  await w.hold('enter');
  assert.equal(w.verbs('answer').length, 0);
  assert.equal(w.verbs('draft').length, 0);
  assert.equal(w.popup.mode, 'group', 'the press itself opened it');
});

test('held keys that act are dropped: every key that sends, opens or confirms', async () => {
  for (const key of ['enter', 'd', 'g', 'y', 'x', 'space', '1', '2', 'tab', 'f', 'o', 'r']) {
    const w = await open(world([group(1, [1, 2]), failed(8), group(9)]));
    const state = JSON.stringify([w.popup.mode, w.popup.view.cursor, w.popup.confirm]);
    w.tick(1000);
    for (let i = 0; i < 40; i += 1) await w.key(key, { held: true, gap: 50 });
    assert.equal(JSON.stringify([w.popup.mode, w.popup.view.cursor, w.popup.confirm]), state, key);
    assert.equal(w.ran.filter(a => a[0] === 'app' && a[1] !== '--help').length, 0, key);
  }
});

test('a held key can\'t close a confirmation or answer one', async () => {
  const w = await open(world([group(1), group(5)]));
  await w.key('d');
  await w.key('y', { held: true });
  await w.key('z', { held: true });
  await w.key('down', { held: true });
  assert.ok(w.popup.confirm, 'still asking');
  assert.equal(w.verbs('dismiss').length, 0);
});

test('held movement moves, and held typing types, but held Enter does not submit the box', async () => {
  const w = await open(world([group(1), group(5)]));
  await w.key('down', { held: true });
  assert.equal(w.popup.view.cursor, 'group:5');
  await w.key('enter');
  await w.key('down');
  await w.key('down');
  await w.key('enter');
  await w.key('a');
  await w.key('a', { held: true });
  assert.equal(w.popup.view.editing.box.chars.join(''), 'aa');
  await w.key('enter', { held: true });
  assert.ok(w.popup.view.editing, 'the box is still open');
  assert.equal(w.verbs('answer').length, 0);
});

test('a click sends the option clicked, whatever the cursor is on', async () => {
  const w = await open(world([three(1), group(5)]));
  await w.key('enter');
  assert.equal(w.popup.view.rows[w.popup.view.cursor], 'opt:0');
  await w.click(2);
  assert.deepEqual(choice(w.verbs('answer')[0]).choices, ['Option C']);
});

test('a click under a confirmation does nothing at all', async () => {
  const w = await open(world([three(1), group(5)]));
  await w.key('enter');
  await w.key('d');
  await w.click(1);
  assert.ok(w.popup.confirm);
  assert.equal(w.verbs('answer').length, 0);
});

test('a click with a box open neither submits nor changes what was typed', async () => {
  const w = await open(world([three(1), group(5)]));
  await w.key('enter');
  await w.key('4');
  await w.key('enter');
  await w.key('h');
  await w.click(0);
  await w.button('enter');
  await w.button('escape');
  assert.equal(w.popup.view.editing.box.chars.join(''), 'h');
  assert.equal(w.ran.filter(a => a[1] === 'answer').length, 0);
});

test('a click on an unarmed view does nothing, one armed acts', async () => {
  const w = await open(world([three(1), group(5)]));
  await w.key('enter');
  w.clock = w.popup.view.armed_at - 1;
  w.view.onRow(2, w.cx);
  await w.settle();
  assert.equal(w.verbs('answer').length, 0);
  w.clock = w.popup.view.armed_at;
  w.view.onRow(2, w.cx);
  await w.settle();
  assert.deepEqual(choice(w.verbs('answer')[0]).choices, ['Option C']);
});

test('a click in the grace window does nothing', async () => {
  const w = await open(world([group(1, [1, 2]), group(5)]));
  await w.key('enter');
  w.db.feed.entries[0].questions.shift(); // the question on screen closed: a grace window opens
  await w.timer();
  w.view.onRow(0, w.cx);
  await w.settle();
  assert.equal(w.ran.filter(a => a[1] === 'draft' || a[1] === 'answer').length, 0);
});

test('a click in a refusal\'s grace window does nothing, though the view is armed', async () => {
  const w = await open(world([three(1), group(5)]));
  await w.key('enter');
  w.db.lock = 'busy';
  await w.key('enter');
  w.db.lock = null;
  w.clock = w.popup.view.armed_at + 10;
  assert.ok(w.clock < w.popup.timers.grace_until);
  const before = w.verbs('answer').length;
  w.view.onRow(2, w.cx);
  await w.settle();
  assert.equal(w.verbs('answer').length, before);
});

test('a click on a held deep link does nothing', async () => {
  const w = await open(world([three(1), group(5)]), { props: { overlay: 'o1', question: 1 } });
  assert.equal(w.popup.held, true);
  await w.click(1);
  assert.equal(w.verbs('answer').length, 0);
});

test('a footer button under a confirmation neither closes it nor sends', async () => {
  const w = await open(world([three(1), group(5)]));
  await w.key('enter');
  await w.key('d');
  await w.button('enter');
  assert.ok(w.popup.confirm);
  assert.equal(w.verbs('dismiss').length, 0);
});

test('a click while a verb runs does nothing', async () => {
  const w = await open(world([group(1, [1, 2]), group(5)]));
  await w.key('enter'); // opens the group
  let release;
  w.gate = new Promise(done => { release = done; });
  w.tick(700);
  w.view.onKey(w.event('enter'), w.cx); // drafts Q1 and moves to Q2
  for (let i = 0; i < 5; i += 1) await new Promise(done => { setImmediate(done); });
  w.tick(700);
  w.view.onRow(1, w.cx);
  release();
  w.gate = null;
  await w.settle();
  assert.equal(w.verbs('draft').length, 1);
});

test('a click on a multi-select toggles that option and sends nothing', async () => {
  const g = three(1);
  g.questions[0].multi_select = true;
  const w = await open(world([g, group(5)]));
  await w.key('enter');
  await w.click(1);
  assert.deepEqual(w.popup.view.picked, [1]);
  assert.equal(w.verbs('answer').length, 0);
});

test('Esc closes the box, goes back, then closes: each one step', async () => {
  const w = await open(world([group(1), group(5)]));
  await w.key('enter');
  await w.key('n');
  await w.key('h');
  await w.escape();
  assert.equal(w.popup.view.editing, null);
  assert.equal(w.popup.mode, 'group');
  await w.escape();
  assert.equal(w.popup.mode, 'list');
  assert.equal(w.closed, false);
  await w.escape();
  assert.equal(w.closed, true);
});

test('Esc held is one Esc', async () => {
  const w = await open(world([group(1), group(5)]));
  await w.key('enter');
  await w.escape();
  for (let i = 0; i < 20; i += 1) {
    w.tick(-660); // 40 ms apart, against escape()'s 700
    await w.escape();
  }
  assert.equal(w.closed, false);
  assert.equal(w.popup.mode, 'list');
});

test('Esc during a verb waits: the result still reaches the person', async () => {
  const w = await open(world([group(1)]), { standin: { host: 'gone' } });
  let release;
  w.gate = new Promise(done => { release = done; });
  w.tick(700);
  w.view.onKey(w.event('enter'), w.cx);
  w.view.onEscape(w.cx);
  for (let i = 0; i < 5; i += 1) await new Promise(done => { setImmediate(done); });
  release();
  w.gate = null;
  await w.settle();
  assert.match(w.toasts, /nothing was sent/);
});

test('the host closing the overlay during a verb says so', async () => {
  const w = await open(world([group(1), group(2)]));
  await w.key('enter');
  let release;
  w.gate = new Promise(done => { release = done; });
  w.tick(700);
  w.view.onKey(w.event('enter'), w.cx);
  for (let i = 0; i < 5; i += 1) await new Promise(done => { setImmediate(done); });
  await w.view.close(w.cx);
  assert.match(w.toasts, /closed before a command finished/);
  release();
});

test('a refusal after the overlay would have closed keeps it open, reloads and tells', async () => {
  const w = await open(world([group(1)]));
  w.db.lock = 'busy';
  await w.key('enter');
  assert.equal(w.closed, false);
  assert.match(w.toasts, /busy/);
  assert.equal(w.popup.mode, 'group');
});

test('after a stale refusal the feed is read at once, and nothing is sent again', async () => {
  const w = await open(world([group(1), group(2)]));
  await w.key('enter');
  w.db.find('group:1').questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  await w.key('enter');
  assert.equal(w.verbs('answer').length, 1);
  assert.equal(w.view.session.pins.changed, false, 'the feed came in: what is on screen is the new row');
  assert.match(w.view.session.change, /stored answer of Q1/);
});

test('the overlay closes when the last answer is recorded, and a warning goes out as a toast', async () => {
  const w = await open(world([group(1)]), { standin: { host: 'gone' } });
  await w.key('enter');
  assert.equal(w.closed, true);
  assert.match(w.toasts, /nothing was sent/);
  assert.equal(/Sending/.test(w.toasts), false);
});

test('held is told as a toast with the overlay still open too', async () => {
  const w = await open(world([group(1), group(5)]), { standin: { host: 'gone' } });
  await w.key('enter');
  await w.key('enter');
  assert.equal(w.closed, false);
  assert.match(w.toasts, /nothing was sent/);
});

test('two pumps at once run one command once', async () => {
  const w = await open(world([group(1), group(2)]));
  await w.key('enter');
  w.tick(700);
  w.view.session.key('enter', w.clock);
  await Promise.all([w.view.pump(w.cx), w.view.pump(w.cx), w.view.pump(w.cx)]);
  assert.equal(w.verbs('answer').length, 1);
});

test('a result that throws in the session reads as unknown, and the keys come back', async () => {
  const w = await open(world([group(1), group(2)]));
  await w.key('enter');
  const finish = w.view.session.finish;
  w.view.session.finish = () => { throw new Error('boom'); };
  await w.key('enter');
  w.view.session.finish = finish;
  assert.match(w.toasts, /unknown/);
  assert.equal(w.view.session.batch, null);
});

test('paste reaches only an open box, and a held paste does nothing', async () => {
  const w = await open(world([group(1), group(5)]));
  w.host.clipboard = 'dy';
  await w.key('v', { control: true });
  assert.equal(w.popup.confirm, null);
  await w.key('enter');
  await w.key('down');
  await w.key('down');
  await w.key('enter');
  await w.key('v', { control: true });
  assert.equal(w.popup.view.editing.box.chars.join(''), 'dy');
  await w.key('v', { control: true, held: true });
  assert.equal(w.popup.view.editing.box.chars.join(''), 'dy');
});

test('--ensure comes at most once per 5 s while the feed is stale', async () => {
  const w = await open(world([group(1), group(2)]));
  const ensures = () => w.ran.filter(a => a[0] === 'app-feed').length;
  const before = ensures();
  w.feedOverride = null; // the feed went missing
  await w.timer();
  await w.timer();
  await w.timer();
  await w.timer();
  assert.ok(ensures() - before <= 2, String(ensures() - before));
  assert.ok(ensures() - before >= 1);
});

test('the first feed is read before --ensure: a readable feed waits for nothing', async () => {
  const w = await open(world([group(1), group(2)]));
  assert.equal(w.readBeforeEnsure, true);
});

test('on a Cyrillic layout a key acts by its place, and types its letter in a box', async () => {
  const w = await open(world([group(1), group(5)]));
  await w.key('d', { char: 'в' });
  assert.ok(w.popup.confirm, 'д on the d key dismisses');
  await w.key('y', { char: 'н' });
  assert.equal(w.verbs('dismiss').length, 1);
  await w.key('j', { char: 'о' });
  assert.equal(w.popup.view.cursor, 'group:5');
  await w.key('enter');
  await w.key('n', { char: 'т' });
  await w.key('h', { char: 'р' });
  assert.equal(w.popup.view.editing.box.chars.join(''), 'р');
});

test('where the host gives the layout\'s letter as the key too, its place is looked up', async () => {
  const w = await open(world([group(1), group(5)]));
  await w.key('в', { char: 'в' });
  assert.ok(w.popup.confirm);
  await w.key('н', { char: 'н' });
  assert.equal(w.verbs('dismiss').length, 1);
});

test('g: core through the app, TUI through the launcher, each saying why it could not', async () => {
  const core = group(1, [1], { asker: asker({ host: 'herdr-core', pane: null, terminal: 't3', tab_label: null }) });
  const w = await open(world([core, group(5)]));
  await w.key('g');
  assert.deepEqual(w.host.ran.at(-1), ['agent.focus', { terminal: 't3' }]);
  assert.equal(w.closed, true);
  const b = await open(world([core, group(5)]), { agents: [{ address: 't3', status: 'blocked' }] });
  await b.key('g', { gap: 100 });
  assert.equal(b.host.ran.length, 0, 'an agent at a prompt needs the pause');
  const t = await open(world([group(5), core]));
  t.openCode = 3;
  await t.key('down');
  await t.key('g');
  assert.deepEqual(t.ran.at(-1), ['open', '--question', 'Q5']);
  assert.match(t.view.session.notices.map(n => n.text).join(), /popup is open already/);
});

test('an incomplete feed that has its last visible row answered says what is left', async () => {
  const w = await open(world([group(1)], { omitted: { ...noOmissions(), groups: 1, questions: 2, question_ids: [21, 22] } }));
  await w.key('enter');
  assert.equal(w.closed, true);
  assert.match(w.toasts, /Still open, and not listed here: 2 questions/);
});
