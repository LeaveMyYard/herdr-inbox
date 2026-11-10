// The safety rules between the feed, the model and the verbs, against the stand-in for B4's verbs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readResult, verbArgs } from '../mods/herdr-inbox/commands.js';
import { tokenOf } from './support/standin.mjs';
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
  for (const bad of [{ code: 9, stdout: '', stderr: '' }, { code: 0, stdout: '', stderr: '' }, null, { code: 1, stdout: JSON.stringify({ v: 1, ok: true, verb: 'answer', entry: 'group:1', token: null, done: [1], skipped: [], delivery: 'queued' }), stderr: '' }]) {
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
  const whole = world([]);
  const { v, ...unversioned } = whole;
  for (const bad of [null, 'not json', ...[{ ...whole, v: 2 }, { ...whole, v: true }, unversioned].map(f => JSON.stringify(f)), JSON.stringify({ ...world([]), ok: false, errors: ['Question Text 345'] })]) {
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
  bad.entries[0].key = 'group:77';
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
  assert.equal(r.popup.confirm, null, 'a refused confirmation does not come back armed');
  r.tick(1000);
  r.press('y');
  assert.equal(r.verbs('dismiss').length, 1, 'another y does not dismiss what changed');
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

test('a held row never reads as sending: its placeholder says nothing was sent', () => {
  const r = new Rig(world([group(1), group(5)]), { standin: { host: 'gone' } });
  r.press('enter', 'enter');
  assert.equal(r.popup.closed.get('group:1'), 'recorded, nothing sent');
});

test('a retry that is held has no sending… either', () => {
  const r = new Rig(world([group(1), group(2), failed(8)]), { standin: { host: 'gone' } });
  r.press('down', 'down', 'enter', 'enter');
  assert.equal(r.popup.closed.get('failed:8'), 'recorded, nothing sent');
});

test('a Submit where every question was skipped says nothing was sent, on the row too', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.press('enter', 'enter', 'enter');
  assert.equal(r.popup.view.kind, 'review');
  r.db.find('group:1').questions.forEach(q => { q.draft = null; });
  r.db.retoken(r.db.find('group:1'));
  r.session.pins.moved('group:1', r.db.find('group:1').token);
  r.press('enter');
  assert.equal(r.popup.closed.get('group:1'), 'nothing sent');
});

test('a core answer is queued like any other', () => {
  const core = group(1, [1], { asker: asker({ host: 'herdr-core', pane: null, terminal: 't3', tab_label: null }) });
  const r = new Rig(world([core, group(5)]), { standin: { server: 'herdr-core' } });
  r.press('enter', 'enter');
  assert.match(r.said, /not delivered yet/);
  assert.equal(/nothing was sent/.test(r.said), false);
});

test('stale closed on one question leaves its siblings open and reachable', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.press('enter');
  r.db.find('group:1').questions.shift();
  r.exec = () => ({ code: 1, stdout: JSON.stringify({ v: 1, ok: false, verb: 'draft', entry: 'group:1', code: 'stale', message: '', closed: 'withdrawn' }), stderr: '' });
  r.press('enter');
  assert.deepEqual(r.session.book.row('group:1').questions.map(q => q.id), [2]);
  assert.equal(r.popup.closed.has('group:1'), false);
});

test('stale closed on a Submit of several says nothing of which one: the feed will', () => {
  const r = new Rig(world([group(1, [1, 2, 3])]));
  r.press('enter', 'enter', 'enter');
  assert.equal(r.popup.view.kind, 'review');
  r.exec = () => ({ code: 1, stdout: JSON.stringify({ v: 1, ok: false, verb: 'answer', entry: 'group:1', code: 'stale', message: '', closed: 'answered' }), stderr: '' });
  r.press('enter');
  assert.equal(r.session.book.row('group:1').questions.length, 3);
});

test('a deep link past the id cap asks show before it says closed', () => {
  const feed = world([], { omitted: { ...noOmissions(), groups: 1500, questions: 1500, question_ids: [1] } });
  const r = new Rig(feed, { link: { question: 5000 }, rows: [group(5000, [5000])] });
  r.db.retoken(r.db.rows[0]);
  assert.equal(r.verbs('show').length, 1);
  assert.equal(r.popup.entry, 'group:5000');
  assert.equal(/no longer open/.test(r.said), false);
});

test('a publish while a deep link\'s show runs does not answer for it', () => {
  const feed = world([], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] } });
  const s = new Rig(world([]));
  s.session.link = { qid: 21, hold: true, tried: true };
  s.session.popup = null;
  s.session.showing = { qid: 21, then: 'open' };
  s.session.read(JSON.stringify({ ...feed, generated_at: s.now / 1000 }), s.now);
  assert.equal(s.popup, null, 'it waits for the answer');
});

test('a deep link whose show failed does not say the question closed', () => {
  const feed = world([], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] } });
  const r = new Rig(world([]));
  r.session.link = { qid: 21, hold: true, tried: true, unread: true };
  r.session.popup = null;
  r.session.feed = feed;
  r.session.start(r.now);
  assert.match(r.said, /could not be read/);
});

test('show answering for another question is of unknown effect, and nothing moves', () => {
  const feed = world([group(1), group(2)], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [9] } });
  const r = new Rig(feed);
  r.exec = () => ({ code: 0, stdout: JSON.stringify({ v: 1, ok: true, verb: 'show', entry: 'group:2', row: r.db.find('group:2') }), stderr: '' });
  r.press('o', 'enter');
  assert.match(r.said, /Q9 could not be read/);
  assert.equal(/may or may not/.test(r.said), false, 'show writes nothing, so nothing may have been done');
  assert.equal(r.popup.view.cursor, 'group:1');
});

test('show with no questions in its row does not throw', () => {
  const feed = world([group(1), group(2)], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [9] } });
  const r = new Rig(feed);
  r.exec = () => ({ code: 0, stdout: JSON.stringify({ v: 1, ok: true, verb: 'show', entry: 'group:9', row: { key: 'group:9', kind: 'group' } }), stderr: '' });
  r.press('o', 'enter');
  assert.match(r.said, /could not be read/);
});

test('a read owed to a left-out row ends when its view does', () => {
  const feed = world([group(1)], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] } });
  const r = new Rig(feed, { rows: [group(21, [21])] });
  r.db.retoken(r.db.rows[0]);
  r.press('escape');
  assert.equal(r.popup.running, false);
  const s = new Rig(world([group(1), group(2)], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] } }), { rows: [group(21, [21])] });
  s.db.retoken(s.db.rows[0]);
  s.press('o', 'enter');
  s.tick(1100);
  s.session.popup.view.cursor = 'group:21';
  s.press('enter');
  s.db.rows[0].questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  s.press('enter');
  assert.ok(s.session.reshow);
  s.tick(1000);
  s.press('escape'); // Esc skips the read and leaves the row
  assert.equal(s.session.reshow, null);
  s.press('down');
  assert.equal(s.verbs('show').length, 1, 'a key elsewhere is that key');
});

test('arrivals into an empty view are reached with Tab, on an incomplete feed too', () => {
  const r = new Rig(world([], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] } }));
  r.db.feed.entries.push(group(1));
  r.published();
  r.press('tab');
  assert.equal(r.popup.mode, 'list');
});

test('what was said goes once the next key is taken', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.db.lock = 'busy';
  r.press('enter', 'enter');
  assert.match(r.said, /busy/);
  r.db.lock = null;
  r.tick(1000);
  r.press('down');
  assert.equal(r.said, '');
});

test('a row with no token and an answer too long are told apart', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.press('enter');
  r.session.pins.open.token = null;
  r.press('enter');
  assert.match(r.said, /does not take that action now/);
  assert.equal(/too long/.test(r.said), false);
});

test('exit 2 says nothing was done: the contract says no I/O', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.press('enter');
  r.exec = () => ({ code: 2, stdout: '', stderr: 'usage' });
  r.press('enter');
  assert.match(r.said, /nothing was done/);
  assert.equal(/may or may not/.test(r.said), false);
});

test('an unknown delivery state offers no action', () => {
  const r = new Rig(world([failed(8, { state: 'future_state' }), group(1), group(2)]));
  r.session.popup.view.cursor = 'failed:8';
  r.press('enter', 'enter');
  assert.equal(r.verbs('retry').length, 0);
});

test('r on a ⚠ row opens it and sends nothing', () => {
  const r = new Rig(world([group(1), group(2), failed(8)]));
  r.press('r');
  assert.equal(r.popup.mode, 'list');
  r.session.popup.view.cursor = 'failed:8';
  r.press('r');
  assert.equal(r.popup.mode, 'failed');
  assert.equal(r.ran.length, 0);
});

test('reads wait while a verb runs', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.press('enter');
  r.session.key('enter', r.now + 700);
  const before = r.session.feed;
  r.session.read(r.db.published(), r.now + 800);
  assert.equal(r.session.feed, before);
  assert.ok(r.session.waiting);
});

test('f never moves the token on, even when show has a newer one', () => {
  const cut = group(1, [1]);
  cut.questions[0].cut = true;
  const r = new Rig(world([cut, group(2)]));
  r.press('enter');
  const pinned = r.session.pins.open.token;
  const newer = structuredClone(r.db.find('group:1'));
  newer.token = 'f'.repeat(32);
  r.db.full = new Map([['group:1', newer]]);
  r.press('f');
  assert.equal(r.session.pins.open.token, pinned);
  assert.equal(r.session.book.row('group:1').token, pinned);
});

test('a left-out ⚠ row or alert is not closed by its absence', () => {
  const alert = { key: 'alert:3', kind: 'alert', id: 3, alert_kind: 'agent', severity: 'info', title: 'Question Title 123', message: null, asked_by: 'lane', created_at: '2026-01-02T10:01:00+00:00', asker: asker(), actions: ['ack'], token: 'x' };
  const r = new Rig(world([group(1), failed(8), alert]));
  r.db.feed.entries = [group(1)];
  r.db.feed.omitted = { ...noOmissions(), failed: 1, alerts: 1 };
  r.published();
  assert.equal(r.popup.closed.has('failed:8'), false);
  assert.equal(r.popup.closed.has('alert:3'), false);
  assert.ok(r.session.book.row('failed:8') && r.session.book.row('alert:3'));
});

test('a kept group holds only the questions the feed still names', () => {
  const r = new Rig(world([group(1), group(21, [21, 22])]));
  r.db.feed.entries = [group(1)];
  r.db.feed.omitted = { ...noOmissions(), groups: 1, questions: 1, question_ids: [22] };
  r.published();
  assert.deepEqual(r.session.book.row('group:21').questions.map(q => q.id), [22]);
});

test('errors alone make a feed incomplete', () => {
  const r = new Rig(world([], { errors: ['Question Text 345'] }));
  assert.equal(r.session.empty, false);
});

test('a delivery word it does not know is of unknown effect', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.press('enter');
  r.exec = () => ({ code: 0, stdout: JSON.stringify({ v: 1, ok: true, verb: 'answer', entry: 'group:1', token: null, done: [1], skipped: [], delivery: 'arrived' }), stderr: '' });
  r.press('enter');
  assert.match(r.said, /unknown/);
});

test('dismissing all at once is one verb only when the row holds just those', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.press('d');
  r.session.book.row('group:1').questions.push({ ...r.session.book.row('group:1').questions[0], id: 3 });
  r.press('y');
  assert.equal(r.verbs('dismiss')[0].includes('--question'), true);
});

test('a feed naming one row twice is refused as a whole', () => {
  const r = new Rig(world([group(1), group(2)]));
  const bad = structuredClone(r.db.feed);
  bad.entries.push(structuredClone(bad.entries[0]));
  r.read(JSON.stringify(bad));
  assert.equal(r.session.problem, 'unreadable');
});

test('a refusal arms the view anew', () => {
  const r = new Rig(world([group(1), group(5)]));
  r.press('enter');
  r.db.lock = 'busy';
  r.press('enter');
  assert.ok(r.popup.view.armed_at > r.now, 'armed after the refusal, not as it was');
});

test('a code it does not know reads as error, and no token is no command', () => {
  const result = { code: 1, stdout: JSON.stringify({ v: 1, ok: false, verb: 'ack', entry: 'alert:1', code: 'future', message: 'x', closed: null }), stderr: '' };
  assert.equal(readResult(result, 'ack', 'alert:1').code, 'error');
  assert.equal(verbArgs({ verb: 'ack', entry: 'alert:1' }, ''), null);
  assert.equal(verbArgs({ verb: 'ack', entry: 'alert:1' }, null), null);
  assert.equal(verbArgs({ verb: 'draft', entry: 'group:1', qids: [0] }, 'x'), null);
});

test('typed text stays through a stale refusal, for a deliberate key after a look', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.press('enter', 'down', 'down', 'enter', 'T', 'x', 't');
  r.db.find('group:1').questions[1].draft = { choices: ['Option B'], notes: {}, text: null };
  r.published();
  r.press('enter'); // the box submits; the draft verb is refused
  assert.match(r.said, /look again/);
  assert.equal(r.popup.view.kind, 'question');
  assert.equal(r.popup.view.editing.box.chars.join(''), 'Txt');
});

test('after a stale on a left-out row, the re-read keeps the view and what was typed', () => {
  const feed = world([group(1), group(2)], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] } });
  const r = new Rig(feed, { rows: [group(21, [21])] });
  r.db.retoken(r.db.rows[0]);
  r.press('o', 'enter');
  r.tick(1100);
  assert.equal(r.popup.view.cursor, 'group:21');
  r.press('enter'); // the lone question opens
  r.press('down', 'down', 'enter', 'h', 'e', 'l', 'l', 'o');
  assert.equal(r.popup.view.editing.box.chars.join(''), 'hello');
  r.db.rows[0].questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  r.db.retoken(r.db.rows[0]);
  r.press('enter'); // refused as stale
  assert.match(r.said, /look again/);
  assert.ok(r.session.reshow);
  r.press('!');
  assert.equal(r.verbs('show').length, 2, 'the row was read again');
  assert.equal(r.popup.mode, 'group', 'still on the question');
  assert.equal(r.popup.view.editing.box.chars.join(''), 'hello!');
  assert.equal(r.verbs('answer').length, 1, 'nothing more was sent');
  r.tick(1000);
  r.press('enter');
  assert.equal(r.verbs('answer').length, 2, 'a deliberate key sends, with the token of the read');
  assert.deepEqual(r.db.delivered.map(d => d.detail.text), ['hello!']);
});

test('a question gone from a group the feed still holds reads closed, not removed', () => {
  const r = new Rig(world([group(1, [1, 2]), group(5)]));
  r.press('enter', 'right');
  assert.equal(r.popup.current, 'q:2');
  r.db.find('group:1').questions.pop();
  r.published();
  assert.equal(r.popup.gone.get('q:2') === 'removed', false);
  assert.ok(r.popup.gone.has('q:2'));
});

test('the book takes the token of the mod\'s own result', () => {
  const r = new Rig(world([group(1, [1, 2])]));
  r.press('enter'); // drafts Q1
  const result = tokenOf(r.db.find('group:1'));
  assert.equal(r.session.book.row('group:1').token, result);
  assert.equal(r.session.pins.changed, false, 'its own write is no change under it');
});

test('after a stale on a left-out row, Enter in the box reads again and sends nothing', () => {
  const feed = world([group(1), group(2)], { omitted: { ...noOmissions(), groups: 1, questions: 1, question_ids: [21] } });
  const r = new Rig(feed, { rows: [group(21, [21])] });
  r.db.retoken(r.db.rows[0]);
  r.press('o', 'enter');
  r.tick(1100);
  r.press('enter', 'down', 'down', 'enter', 'h', 'i');
  r.db.rows[0].questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  r.db.retoken(r.db.rows[0]);
  r.press('enter'); // refused as stale
  const before = r.ran.filter(a => a[1] !== 'show').length;
  r.press('enter'); // pressed before the re-read was seen
  assert.equal(r.ran.filter(a => a[1] !== 'show').length, before, 'no verb');
  assert.equal(r.popup.view.editing.box.chars.join(''), 'hi');
});
