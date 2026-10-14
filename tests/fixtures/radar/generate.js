'use strict';
// Regenerates the herdr-radar sidebar-block fixtures from radar's own generator
// (lib/managed-config.js sidebarBlock), so the tests edit exactly what radar writes.
//
//   node tests/fixtures/radar/generate.js ~/.config/herdr/plugins/github/hhdebb.herdr-radar-<hash>
//
// Each case runs in a child process with its own radar config dir (glyph variant) and
// herdr state dir (the agent ids rows_by_agent may name). Nothing outside .scratch/ and
// this folder is written.
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const radar = path.resolve(process.argv[2]);
const here = __dirname;
const work = path.join(here, '..', '..', '..', '.scratch', 'radar-fixtures');

if (process.argv[3] === '--child') {
  const mc = require(path.join(radar, 'lib', 'managed-config.js'));
  process.stdout.write(mc.sidebarBlock(process.argv[4]));
  process.exit(0);
}

const cases = [
  // name, appearance, glyph variant, agent ids herdr knows
  ['dark-font', 'dark', 'font', ['claude', 'gemini', 'kimi', 'qwen']],
  ['light-text', 'light', 'text', ['claude', 'codex']],
  ['dark-no-vendors', 'dark', 'none', []],
];
for (const [name, appearance, variant, agents] of cases) {
  const cfg = path.join(work, name, 'config');
  const state = path.join(work, name, 'state');
  const remote = path.join(state, 'herdr', 'agent-detection', 'remote');
  fs.mkdirSync(cfg, { recursive: true });
  fs.mkdirSync(remote, { recursive: true });
  fs.writeFileSync(path.join(cfg, 'config.toml'), `variant = "${variant}"\n`);
  for (const id of agents) fs.writeFileSync(path.join(remote, `${id}.toml`), '');
  const block = execFileSync(process.execPath, [__filename, radar, '--child', appearance], {
    env: { ...process.env, HERDR_PLUGIN_CONFIG_DIR: cfg, XDG_STATE_HOME: state },
  }).toString();
  fs.writeFileSync(path.join(here, `${name}.block.toml`), block + '\n');
  console.log(name, block.length);
}
