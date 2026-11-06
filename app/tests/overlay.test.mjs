import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./support/loader.mjs', import.meta.url);
const { default: HerdrInbox, words } = await import('../mods/herdr-inbox/main.js');
const { world } = await import('./support/herdr.mjs');

// Runs init and its task to the end, as the host would, and gives the text of each render.
async function open() {
  const view = new HerdrInbox();
  const tasks = [];
  const cx = { spawn: task => tasks.push(task), notify() {}, focus_handle: () => ({ focus() {} }), timer: { every() {} } };
  view.init({}, cx);
  const first = view.render().texts;
  // The feed watcher never ends: a task gets a moment, not a wait for its end.
  const moment = () => new Promise(done => { setTimeout(done, 5); });
  while (tasks.length > 0) await Promise.race([tasks.shift()(cx), moment()]);
  await moment();
  return [first, view.render().texts];
}

const RECEIPT = `${world.appHome}/data/herdr-inbox/install.json`;
const ROOT = '/fake/plugin';

function installed(app) {
  world.files = {
    [RECEIPT]: JSON.stringify({ v: 1, plugin_root: ROOT, plugin_moves: false, claude_config: null }),
    [`${ROOT}/herdr-plugin.toml`]: 'id = "leavemyyard.herdr-inbox"\n',
    [`${ROOT}/scripts/launch`]: '#\n',
  };
  world.exec = async (_command, args) => (args.at(-1) === 'which' ? { code: 0, stdout: '/fake/bin\n', stderr: '' } : app);
}

test('the overlay checks first, and then says what it found', async () => {
  installed({ code: 0, stdout: 'herdr-inbox app v1\n', stderr: '' });
  const [first, last] = await open();
  assert.deepEqual(first, ['Inbox', 'Looking for the inbox…']);
  assert.equal(last[0], 'Inbox unavailable');
  assert.match(last[1], /no inbox feed/, 'no feed yet: never an empty inbox');
});

test('app exiting 2 shows that the inbox is unavailable, and why', async () => {
  installed({ code: 2, stdout: '', stderr: 'usage: inbox' });
  const [, last] = await open();
  assert.equal(last[0], 'Inbox unavailable');
  assert.match(last[1], /no app commands/);
});

test('a home with no install says so too', async () => {
  world.files = {};
  const [, last] = await open();
  assert.equal(last[0], 'Inbox unavailable');
  assert.match(last[1], /install record/);
});

test('--ensure runs only after availability says ready, and nothing runs hc', async () => {
  const calls = [];
  installed({ code: 2, stdout: '', stderr: 'usage: inbox' });
  const refuse = world.exec;
  world.exec = async (command, args) => { calls.push([command, ...args]); return refuse(command, args); };
  await open();
  assert.equal(calls.some(c => c.includes('--ensure')), false, 'not ready: nothing is started');
  installed({ code: 0, stdout: 'herdr-inbox app v1\n', stderr: '' });
  const ready = world.exec;
  calls.length = 0;
  world.exec = async (command, args) => { calls.push([command, ...args]); return ready(command, args); };
  await open();
  const ensures = calls.filter(c => c.includes('--ensure'));
  assert.deepEqual(ensures.map(c => c.slice(2)), [['cli', 'app-feed', '--ensure', '--app-home', world.appHome], ['cli', 'worker', '--ensure']]);
  assert.ok(calls.findIndex(c => c.includes('--help')) < calls.findIndex(c => c.includes('--ensure')));
  assert.equal(calls.some(c => c[0] === 'hc'), false);
});

test('every reason has its words: an unavailable inbox never renders blank', () => {
  for (const reason of ['not-installed', 'no-plugin', 'no-app']) {
    assert.ok(words({ state: 'unavailable', reason }).detail.length > 20, reason);
  }
});
