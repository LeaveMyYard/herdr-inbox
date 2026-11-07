// Finds the inbox plugin and asks whether its `app` commands work. The mod has no binary of its
// own: every command goes through the plugin's launcher, so `engine` and the contract check apply.
//
// `host` is the `herdr` module, or what a test gives instead.

const PLUGIN_ID = 'leavemyyard.herdr-inbox';
const CLAUDE_PLUGIN = 'herdr-inbox@herdr-inbox';
const RECEIPT = 'data/herdr-inbox/install.json';

async function read(host, path) {
  try {
    return await host.readTextFile(path);
  } catch {
    return null;
  }
}

function parsed(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

// The launcher, and the manifest line that says this folder is the inbox and not another plugin.
async function isPlugin(host, root) {
  if (typeof root !== 'string' || !root.startsWith('/')) return false;
  const manifest = await read(host, `${root}/herdr-plugin.toml`);
  if (!manifest?.split('\n').some(line => line.trim() === `id = "${PLUGIN_ID}"`)) return false;
  return (await read(host, `${root}/scripts/launch`)) !== null;
}

// herdr's answer. It needs no server, but it waits on herdr's plugin lock, so while another
// `herdr plugin` command holds that, this can take up to `exec`'s 30 s. With no herdr it rejects.
async function fromHerdr(host) {
  try {
    const result = await host.exec('herdr', ['plugin', 'list', '--json']);
    if (result.code !== 0) return null;
    const plugins = parsed(result.stdout)?.result?.plugins;
    if (!Array.isArray(plugins)) return null;
    return plugins.find(plugin => plugin?.plugin_id === PLUGIN_ID)?.plugin_root ?? null;
  } catch {
    return null;
  }
}

// Claude Code's own record of where the user-scope copy is. The install's shell may not have had
// the CLAUDE_CONFIG_DIR that Claude Code runs with, so the default folder is read as well.
async function fromClaude(host, config) {
  for (const folder of [config, `${host.home()}/.claude`]) {
    if (!folder) continue;
    const entries = parsed(await read(host, `${folder}/plugins/installed_plugins.json`))?.plugins?.[CLAUDE_PLUGIN];
    const path = Array.isArray(entries) ? entries.find(entry => entry?.scope === 'user')?.installPath : null;
    if (path) return path;
  }
  return null;
}

// The plugin root for this start, or null. The operator's own checkout does not move: a pull
// updates it in place, and an app home installed from it stays on it. Everything else moves on an
// update: herdr's installs are checkouts too, in a new generation each time, and the old one can
// stay on disk with an `engine` a rollback never reaches. So what herdr and Claude Code say now
// comes before what the install recorded.
async function resolveRoot(host, receipt) {
  if (!receipt.plugin_moves) return (await isPlugin(host, receipt.plugin_root)) ? receipt.plugin_root : null;
  for (const root of [await fromHerdr(host), await fromClaude(host, receipt.claude_config), receipt.plugin_root]) {
    if (await isPlugin(host, root)) return root;
  }
  return null;
}

// The program and arguments for `herdr-inbox <args>`, for `exec`.
export function launcher(root, args) {
  return ['sh', [`${root}/scripts/launch`, 'cli', ...args]];
}

// app-commands v1: exit 0 and this exact first line. After a rollback, or with no binary, `app`
// reaches Python, which exits 2. A binary older than `app` prints something else. With no binary
// that is known without starting Python, which would make a `.venv` in the plugin on each open.
async function hasApp(host, root) {
  try {
    const binary = await host.exec('sh', [`${root}/scripts/launch`, 'which']);
    if (binary.code !== 0 || binary.stdout.trim() === '') return false;
    const result = await host.exec(...launcher(root, ['app', '--help']));
    return result.code === 0 && result.stdout.split('\n')[0] === 'herdr-inbox app v1';
  } catch {
    return false;
  }
}

// Makes sure the feed writer and the delivery worker run. Call this only after `availability` says
// ready, never directly in `init`: `herdr-app check` runs `init`, and nothing there may start a core.
export async function ensure(host, root) {
  const writer = await host.exec(...launcher(root, ['app-feed', '--ensure', '--app-home', host.appHome()]));
  const worker = await host.exec(...launcher(root, ['worker', '--ensure']));
  return writer.code === 0 && worker.code === 0;
}

// { state: 'ready', root }, or { state: 'unavailable', reason }.
export async function availability(host) {
  const receipt = parsed(await read(host, `${host.appHome()}/${RECEIPT}`));
  if (receipt?.v !== 1) return { state: 'unavailable', reason: 'not-installed' };
  const root = await resolveRoot(host, receipt);
  if (root === null) return { state: 'unavailable', reason: 'no-plugin' };
  if (!(await hasApp(host, root))) return { state: 'unavailable', reason: 'no-app' };
  return { state: 'ready', root };
}
