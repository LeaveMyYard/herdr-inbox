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
  view.init({}, { spawn: task => tasks.push(task), notify() {} });
  const first = view.render().texts;
  const cx = { notify() {} };
  for (const task of tasks) await task(cx);
  return [first, view.render().texts];
}

const RECEIPT = `${world.appHome}/data/herdr-inbox/install.json`;
const ROOT = '/fake/plugin';

function installed(app) {
  world.files = {
    [RECEIPT]: JSON.stringify({ v: 1, plugin_root: ROOT, plugin_copy: false, claude_config: null }),
    [`${ROOT}/herdr-plugin.toml`]: 'id = "leavemyyard.herdr-inbox"\n',
    [`${ROOT}/scripts/launch`]: '#\n',
  };
  world.exec = async () => app;
}

test('the overlay checks first, and then says what it found', async () => {
  installed({ code: 0, stdout: 'herdr-inbox app v1\n', stderr: '' });
  const [first, last] = await open();
  assert.deepEqual(first, ['Inbox', 'Looking for the inbox…']);
  assert.equal(last[0], 'Inbox');
  assert.match(last[1], /can be reached/);
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

test('every reason has its words: an unavailable inbox never renders blank', () => {
  for (const reason of ['not-installed', 'no-plugin', 'no-app']) {
    assert.ok(words({ state: 'unavailable', reason }).detail.length > 20, reason);
  }
});
