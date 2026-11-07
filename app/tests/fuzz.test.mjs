// Random keys, held keys, clicks and feed changes against the real main.js and the stand-in. The
// oracle: no verb without a deliberate press or click, none for a question that was not on screen,
// none picking a row other than the one chosen, and every token the one shown or the mod's own result.
import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./support/loader.mjs', import.meta.url);
const { default: HerdrInbox } = await import('../mods/herdr-inbox/main.js');
const { Window } = await import('./support/host.mjs');
const { group, world } = await import('./support/rig.mjs');

const RUNS = Number(process.env.FUZZ_RUNS ?? 40);
const STEPS = 60;

function random(seed) {
  let s = seed >>> 0;
  return n => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return Math.floor((s / 2 ** 32) * n);
  };
}

const KEYS = ['enter', 'enter', 'down', 'up', 'tab', 'left', 'right', '1', '2', '3', 'space', 'd', 'y', 'n', 'x', 'g', 'a', 'j', 'k'];
const argOf = (args, flag) => args[args.indexOf(flag) + 1];

function shownQuestions(session) {
  const p = session.popup;
  const q = [];
  if (p.view.kind === 'question') q.push(p.view.qid);
  if (p.view.kind === 'review') q.push(...p.view.items);
  if (p.confirm) q.push(...(p.confirm.target.qids ?? [p.confirm.target.qid]));
  return q.filter(id => id !== undefined);
}

function backgroundChange(w, r) {
  const rows = w.db.feed.entries.filter(e => e.kind === 'group');
  const row = rows[r(Math.max(rows.length, 1))];
  const what = r(5);
  if (what === 0) w.db.feed.entries.push(group(100 + r(50)));
  else if (what === 1 && row) w.db.remove(row.key, null);
  else if (what === 2 && row) row.questions[0].draft = { choices: ['Option B'], notes: {}, text: null };
  else if (what === 3 && row && row.questions.length > 1) row.questions.pop();
  if (w.db.feed.entries.filter((e, i, all) => all.findIndex(x => x.key === e.key) !== i).length > 0) {
    w.db.feed.entries = w.db.feed.entries.filter((e, i, all) => all.findIndex(x => x.key === e.key) === i);
  }
}

let verbsRun = 0;

async function run(seed) {
  const r = random(seed);
  const w = await new Window(HerdrInbox, world([group(1, [1, 2]), group(5), group(7, [7, 8, 9])])).opened();
  const session = w.view.session;
  const seen = new Set();
  const real = w.db.run.bind(w.db);
  let allowed = 0; // a deliberate press or click may start verbs, nothing else may
  let shown = [];
  let chosen = null;
  w.db.run = args => {
    const verb = args[1];
    if (['answer', 'draft', 'dismiss', 'retry', 'stop-showing', 'ack'].includes(verb)) {
      assert.ok(allowed > 0, `seed ${seed}: ${verb} with no deliberate action`);
      const token = argOf(args, '--token');
      const pinned = session.pins.token(argOf(args, '--entry'));
      assert.equal(token, pinned, `seed ${seed}: a token that is not the one shown`);
      assert.ok(seen.has(token), `seed ${seed}: a token the mod never held`);
      verbsRun += 1;
      if (verb === 'answer' || verb === 'draft') {
        const asked = args.flatMap((a, i) => (a === '--question' ? [Number(args[i + 1])] : []));
        for (const q of asked) assert.ok(shown.includes(q), `seed ${seed}: Q${q} was not on screen`);
        if (chosen !== null && args.includes('--detail')) assert.deepEqual(JSON.parse(argOf(args, '--detail')).choices, [chosen], `seed ${seed}: not the row clicked`);
      }
    }
    const out = real(args);
    if (out.stdout) {
      const { token } = JSON.parse(out.stdout);
      if (token) seen.add(token);
    }
    return out;
  };
  const remember = () => {
    for (const row of session.book.rows.values()) if (row.token) seen.add(row.token);
    for (const pin of [session.pins.open, session.pins.confirm]) if (pin?.token) seen.add(pin.token);
  };
  for (let i = 0; i < STEPS && !w.closed && session.popup?.running !== false; i += 1) {
    remember();
    shown = shownQuestions(session);
    chosen = null;
    const roll = r(10);
    if (roll < 2) {
      backgroundChange(w, r);
      await w.timer();
    } else if (roll < 4) {
      allowed = 0;
      await w.key(KEYS[r(KEYS.length)], { held: true, gap: 40 + r(600) });
    } else if (roll < 6 && session.popup.view.kind === 'question') {
      const v = session.popup.view;
      const row = r(v.labels.length);
      const plain = !v.editing && !session.popup.confirm && !v.multi_select;
      allowed = 1;
      chosen = plain ? v.labels[row] : null;
      if (!plain) allowed = v.multi_select || v.editing || session.popup.confirm ? 0 : 1;
      await w.click(row);
    } else {
      allowed = 1;
      await w.key(KEYS[r(KEYS.length)], { gap: 100 + r(900) });
    }
  }
}

test(`${RUNS} random sessions of keys, repeats, clicks and feed changes`, async () => {
  for (let seed = 1; seed <= RUNS; seed += 1) await run(seed * 2654435761);
  assert.ok(verbsRun >= RUNS, `only ${verbsRun} verbs ran: the fuzz is not reaching them`);
  console.log(`${verbsRun} verbs in ${RUNS} runs`);
});
