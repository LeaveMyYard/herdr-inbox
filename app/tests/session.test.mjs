// The safety rules between the feed, the model and the verbs, against the stand-in for B4's verbs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { Rig, asker, failed, fixture, group, noOmissions, world } from './support/rig.mjs';

const tokenArg = args => args[args.indexOf('--token') + 1];
const entryArg = args => args[args.indexOf('--entry') + 1];
const twoGroups = () => world([group(1), group(5)]);

test('an answer goes to the question under the cursor when the list changes under it', () => {
  const r = new Rig(twoGroups());
  r.press('down'); // on group:5
  assert.equal(r.popup.view.cursor, 'group:5');
  r.db.feed.entries = [group(5), group(3), group(9)]; // group:1 went, others arrived: every index moved
  r.published();
  r.press('enter', 'enter');
  assert.equal(r.popup.mode === 'list' || !r.popup.running, true);
  const [args] = r.verbs('answer');
  assert.equal(entryArg(args), 'group:5');
  assert.deepEqual(args.slice(args.indexOf('--question'), args.indexOf('--question') + 2), ['--question', '5']);
  assert.deepEqual(r.db.delivered.map(d => d.id), [5]);
});

test('a row that arrived does not take Enter or a digit for its first second', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.db.feed.entries.push(group(7));
  r.published();
  r.tick(10);
  r.session.key('3', r.now); // the digit of the new row
  assert.equal(r.popup.view.cursor, 'group:1');
});

test('keys are dropped while a verb runs, so nothing queues behind it', () => {
  const r = new Rig(world([group(1, [1, 2])]));
  const real = r.exec;
  r.exec = args => {
    r.tick(1000);
    r.session.key('enter', r.now); // while the draft runs
    return real(args);
  };
  r.press('enter'); // the only group opened by itself: its first option drafts
  assert.equal(r.ran.length, 1);
  assert.equal(r.popup.current, 'q:2', 'the key in flight picked nothing on the next tab');
});

test('a verb carries the token of what was opened, not of a newer feed', () => {
  const r = new Rig(twoGroups());
  r.press('enter');
  const opened = r.session.pins.open.token;
  r.db.find('group:1').questions[0].draft = { choices: ['Option B'], notes: {}, text: null }; // the popup stored a draft
  r.published();
  assert.equal(r.session.pins.changed, true);
  r.press('enter');
  const [args] = r.verbs('answer');
  assert.equal(tokenArg(args), opened);
  assert.notEqual(tokenArg(args), r.session.book.row('group:1').token);
  assert.match(r.said, /changed, look again/);
  assert.match(r.said, /Nothing was sent/);
  assert.equal(r.db.delivered, undefined);
});

test('stale: no resubmission by itself, and the next key is not the one in flight', () => {
  const r = new Rig(twoGroups());
  r.press('enter');
  r.db.find('group:1').questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  r.published();
  r.press('enter');
  assert.equal(r.verbs('answer').length, 1, 'one try');
  r.pump();
  r.published();
  assert.equal(r.verbs('answer').length, 1, 'still one after feeds');
  r.session.key('enter', r.now); // a key already on its way: inside the grace window
  r.pump();
  assert.equal(r.verbs('answer').length, 1, 'a key in flight does nothing');
  assert.equal(r.popup.mode, 'group', 'the row is on screen to look at');
  assert.match(r.session.change, /stored answer of Q1 changed/);
});

test('stale with closed says what closed it, and the row is inert', () => {
  const r = new Rig(twoGroups());
  r.press('enter');
  r.db.remove('group:1', 'answered');
  r.press('enter');
  assert.match(r.said, /answered/);
  assert.equal(r.popup.view.kind, 'gone');
  assert.equal(r.verbs('answer').length, 1);
  r.tick(700);
  r.press('enter');
  assert.equal(r.verbs('answer').length, 1, 'a placeholder takes no Enter');
});

test('after a look, the deliberate key sends with the token of the new look', () => {
  const r = new Rig(twoGroups());
  r.press('enter');
  r.db.find('group:1').questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  r.published();
  r.press('enter');
  r.tick(1000);
  r.press('enter');
  assert.equal(r.verbs('answer').length, 2);
  assert.deepEqual(r.db.delivered.map(d => d.id), [1]);
});

test('held says nothing was sent, and never that it is sending', () => {
  const r = new Rig(world([group(1)]), { standin: { host: 'gone' } });
  const toasts = [];
  r.press('enter');
  toasts.push(...r.session.toasts);
  assert.match(toasts.map(t => t.text).join(), /nothing was sent/);
  assert.equal(/Sending/.test(r.said), false);
  assert.equal(r.session.closing, true, 'the toast outlives the overlay');
});

test('queued never claims that it arrived', () => {
  const r = new Rig(twoGroups());
  r.press('enter', 'enter');
  assert.match(r.said, /not delivered yet/);
});

test('a retry that comes out held says nothing could be sent', () => {
  const r = new Rig(world([failed(8), group(1), group(2)]), { standin: { host: 'gone' } });
  r.press('down', 'down', 'enter', 'enter');
  assert.equal(r.verbs('retry').length, 1);
  assert.match(r.said, /Nothing was sent for Q8/);
});

test('a result of unknown effect is said, reloaded and not repeated', () => {
  for (const bad of [{ code: 9, stdout: '', stderr: '' }, { code: 0, stdout: '', stderr: '' }, null, { code: 1, stdout: '{"v":1,"ok":true,"verb":"answer","entry":"group:1"}', stderr: '' }]) {
    const r = new Rig(twoGroups());
    r.press('enter');
    let n = 0;
    r.exec = () => { n += 1; return bad; };
    r.press('enter');
    assert.equal(n, 1);
    assert.match(r.said, /unknown/);
    assert.equal(r.popup.mode, 'group');
  }
});

test('a result naming another row is of unknown effect', () => {
  const r = new Rig(twoGroups());
  r.press('enter');
  r.exec = () => ({ code: 0, stdout: JSON.stringify({ v: 1, ok: true, verb: 'answer', entry: 'group:5', token: null, done: [5], skipped: [], delivery: 'queued' }), stderr: '' });
  r.press('enter');
  assert.match(r.said, /unknown/);
  assert.ok(r.session.book.row('group:5'), 'the book believes nothing of it');
});

test('busy keeps the operator\'s input', () => {
  const r = new Rig(world([group(1)]));
  r.press('down', 'down', 'enter', 'T', 'x', 't');
  r.db.lock = 'busy';
  r.press('enter');
  assert.match(r.said, /try again/);
  assert.equal(r.popup.view.kind, 'question');
  assert.equal(r.popup.view.editing.box.chars.join(''), 'Txt');
  r.db.lock = null;
});

test('an empty feed with a left-out group never reads as an empty inbox', () => {
  const r = new Rig(fixture('omitted-only'), { rows: [group(21, [21, 22])] });
  assert.equal(r.popup.mode, 'empty');
  assert.equal(r.session.empty, false);
  assert.ok(r.session.incomplete.counts.join().includes('2 questions'));
  r.press('x');
  assert.equal(r.popup.running, true, 'any key does not close what is not empty');
});

test('a left-out question opens with show, and its token answers it', () => {
  const left = group(21, [21]);
  const feed = fixture('omitted-only');
  feed.omitted = { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] };
  const r = new Rig(feed, { rows: [left] });
  r.db.retoken(r.db.rows[0]);
  r.press('o', 'enter');
  assert.equal(r.verbs('show').length, 1);
  assert.deepEqual(r.popup.entries, ['group:21']);
  r.tick(1100);
  r.press('enter', 'enter');
  assert.equal(r.verbs('answer').length, 1);
  assert.deepEqual(r.db.delivered.map(d => d.id), [21]);
});

test('absence from a feed is not closed while its id is left out', () => {
  const r = new Rig(world([group(1), group(21), group(30)]));
  r.press('enter');
  const left = r.db.feed.entries.splice(1, 1);
  r.db.rows = left;
  r.db.feed.omitted = { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] };
  r.published();
  assert.equal(r.popup.closed.has('group:21'), false);
  assert.ok(r.session.book.row('group:21').left_out);
  r.db.feed.omitted = noOmissions();
  r.published();
  assert.equal(r.popup.closed.has('group:21'), true, 'a whole feed that does not name it says it closed');
});

test('past the 1000 ids nothing says a missing group closed', () => {
  const r = new Rig(world([group(1), group(21)]));
  r.db.feed.entries.splice(1, 1);
  r.db.feed.omitted = { ...noOmissions(), groups: 2, questions: 2000, question_ids: [100] };
  r.published();
  assert.equal(r.popup.closed.has('group:21'), false);
  assert.equal(r.session.incomplete.unlisted, 1999);
});

test('a stale left-out row is read again on the next key, never by itself', () => {
  const feed = world([]);
  feed.omitted = { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] };
  const r = new Rig(feed, { rows: [group(21, [21])] });
  r.db.retoken(r.db.rows[0]);
  r.press('o', 'enter');
  r.tick(1100);
  r.press('enter');
  r.db.rows[0].questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  r.press('enter');
  assert.equal(r.verbs('show').length, 1, 'the refusal did not read it again');
  r.published();
  assert.equal(r.verbs('show').length, 1, 'neither did a feed');
  r.tick(700);
  r.press('down');
  assert.equal(r.verbs('show').length, 2);
});

test('a cut question says so, and f reads it in full under the same token', () => {
  const cut = group(1, [1]);
  cut.questions[0].cut = true;
  const r = new Rig(world([cut, group(2)]));
  const whole = structuredClone(r.db.find('group:1'));
  whole.questions[0].question = 'Question Text 345 in full';
  whole.questions[0].cut = false;
  r.db.full = new Map([['group:1', whole]]);
  r.press('enter', 'f');
  assert.equal(r.verbs('show').length, 1);
  assert.equal(r.session.full.get(1).question, 'Question Text 345 in full');
  assert.equal(r.session.pins.changed, false);
});

test('a feed older than the mod\'s own write does not bring the row back', () => {
  const r = new Rig(world([group(1, [1, 2])]));
  const old = JSON.stringify({ ...r.db.feed, generated_at: r.now / 1000 - 30, entries: structuredClone(r.db.feed.entries) });
  r.press('enter'); // a draft
  r.read(old);
  assert.ok(r.session.book.row('group:1').questions[0].draft, 'the draft stays');
});

test('a bad or missing feed keeps the last good one and marks it stale', () => {
  const r = new Rig(twoGroups());
  for (const bad of [null, 'not json', '{"v":2}', '{"v":true}', JSON.stringify({ ...world([]), ok: false, errors: ['Question Text 345'] })]) {
    r.read(bad);
    assert.equal(r.session.stale, true, String(bad));
    assert.equal(r.session.book.rows.size, 2);
    assert.equal(r.session.empty, false);
  }
  r.published();
  assert.equal(r.session.stale, false);
});

test('with no feed at all there is no popup, and so no empty inbox', () => {
  const r = new Rig(world([]));
  const fresh = new Rig(fixture('not-migrated'));
  assert.equal(fresh.popup, null);
  assert.equal(fresh.session.problem, 'not-ok');
  assert.equal(r.session.empty, true);
});

test('an old feed is stale but stays', () => {
  const r = new Rig(twoGroups());
  r.tick(16000);
  r.read(JSON.stringify({ ...JSON.parse(r.db.published()), generated_at: r.now / 1000 - 16 }));
  assert.equal(r.session.stale, true);
  r.read(JSON.stringify({ ...JSON.parse(r.db.published()), generated_at: r.now / 1000 + 5 }));
  assert.equal(r.session.stale, true, 'a feed from the future is stale too');
});

test('a row whose key and contents disagree is refused as a whole feed', () => {
  const r = new Rig(twoGroups());
  const bad = structuredClone(r.db.feed);
  bad.entries[0].key = 'group:5';
  r.read(JSON.stringify(bad));
  assert.equal(r.session.problem, 'unreadable');
  assert.equal(r.session.book.rows.size, 2);
});

test('unknown vocabulary stays visible and offers nothing', () => {
  const r = new Rig(fixture('unknown-vocabulary'));
  assert.deepEqual(r.session.book.unknown().map(e => e.key), ['future:1']);
  assert.equal(r.popup.entries.includes('future:1'), false);
  assert.equal(r.session.empty, false);
});

test('a row without the verb in its actions gets no verb', () => {
  const r = new Rig(world([group(1, [1], { actions: ['dismiss'] }), group(2)]));
  r.press('enter', 'enter');
  assert.equal(r.verbs('answer').length, 0);
  assert.match(r.said, /does not take that action/);
});

test('a remote row is read-only: no verb runs, and it says so', () => {
  const feed = fixture('every-kind');
  const r = new Rig(feed);
  const key = feed.entries.find(e => e.kind === 'remote_group').key;
  r.session.popup.view.cursor = key;
  r.press('d', 'y');
  assert.equal(r.ran.length, 0);
  assert.match(r.said, /read-only/);
});

test('a whole group dismissed is one verb, and needs y', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.press('d', 'n');
  assert.equal(r.ran.length, 0);
  r.press('d', 'y');
  const [args] = r.verbs('dismiss');
  assert.equal(r.verbs('dismiss').length, 1);
  assert.equal(args.includes('--question'), false);
  assert.deepEqual(r.db.find('group:1'), undefined);
});

test('the confirmation keeps the token of the moment d was pressed', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.press('d');
  r.db.find('group:1').questions.pop();
  r.published();
  r.press('y');
  assert.match(r.said, /look again/);
  assert.ok(r.db.find('group:1'), 'nothing was dismissed');
});

test('drafts chain the tokens of their own results', () => {
  const r = new Rig(world([group(1, [1, 2])]));
  r.press('enter', 'enter');
  const drafts = r.verbs('draft');
  assert.equal(drafts.length, 2);
  r.press('enter');
  assert.deepEqual(r.db.delivered.map(d => d.id), [1, 2]);
  assert.equal(r.said.includes('look again'), false);
});

test('a detail too long for one argument is refused here, and nothing is sent', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.press('enter', 'down', 'down', 'enter');
  for (const c of Array(70000).fill('x')) r.session.key(c, r.now);
  r.press('enter');
  assert.equal(r.verbs('answer').length, 0);
  assert.match(r.said, /too long/);
});

test('a paste reaches an open text box and nothing else', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.tick();
  r.session.paste(['d', 'y'], r.now);
  assert.equal(r.popup.confirm, null);
  assert.equal(r.ran.length, 0);
  r.press('enter', 'down', 'down', 'enter');
  r.session.paste(['d', 'y'], r.now);
  assert.equal(r.popup.view.editing.box.chars.join(''), 'dy');
});

test('a warning already told is not told again when the overlay closes', () => {
  const r = new Rig(world([group(1)]), { standin: { host: 'gone' } });
  r.press('enter');
  assert.equal(r.session.toasts.length, 1);
});

test('g goes by host: core by terminal, TUI by question, unknown nowhere', () => {
  const core = group(1, [1], { asker: asker({ host: 'herdr-core', pane: null, terminal: 't3', tab_label: null }) });
  const none = group(5, [5], { asker: asker({ host: 'future-host', pane: null }) });
  const r = new Rig(world([core, group(2), none]));
  r.press('g');
  assert.deepEqual(r.ran.at(-1).pane, 'core:t3');
  const s = new Rig(world([core, group(2)]));
  s.press('down', 'g');
  assert.deepEqual(s.ran.at(-1).pane, 'tui:2');
  const u = new Rig(world([none, group(8)]));
  u.press('g');
  assert.equal(u.ran.length, 0);
  assert.match(u.said, /no pane/);
});

test('a deep link opens its question held; a left-out one is shown first', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]), { link: { question: 2 } });
  assert.equal(r.popup.current, 'q:2');
  assert.equal(r.popup.held, true);
  r.press('enter');
  assert.equal(r.ran.length, 0, 'held: no answer without a deliberate Tab');
  const feed = world([]);
  feed.omitted = { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] };
  const s = new Rig(feed, { link: { question: 21 }, rows: [group(21, [21])] });
  s.db.retoken(s.db.rows[0]);
  assert.equal(s.verbs('show').length, 1);
  assert.equal(s.popup.entry, 'group:21');
});

test('a deep link to a closed question says so', () => {
  const r = new Rig(world([group(1)]), { link: { question: 9 } });
  assert.match(r.said, /Q9 is no longer open/);
});

test('both hosts answer with the same verbs', () => {
  const core = group(14, [14], { asker: asker({ host: 'herdr-core', pane: null, terminal: 't3', tab_label: null }) });
  const r = new Rig(world([group(1), core]), { standin: { server: 'herdr-core' } });
  r.press('enter', 'enter');
  r.tick(1200);
  r.press('down', 'enter', 'enter');
  assert.deepEqual(r.db.delivered.map(d => d.id), [1, 14]);
});
