#!/usr/bin/env node
// The stand-in behind a fake plugin's `scripts/launch`, for a real window in a scratch app home:
// `standin-cli.mjs <state file> <launcher's arguments>`. The state holds { feed, rows, options, home }.
// It publishes the feed as the writer would, after every verb and every `--ensure`.
import { readFileSync, renameSync, writeFileSync } from 'node:fs';
import { StandIn } from './standin.mjs';

const [statePath, ...args] = process.argv.slice(2);
const state = JSON.parse(readFileSync(statePath, 'utf8'));
const db = new StandIn(state.feed, { ...state.options, rows: state.rows });

function publish() {
  db.clock = Date.now() / 1000 - 1;
  const target = `${state.home}/data/herdr-inbox/feed.json`;
  writeFileSync(`${target}.tmp`, db.published());
  renameSync(`${target}.tmp`, target);
}

function save() {
  writeFileSync(statePath, JSON.stringify({ ...state, feed: db.feed, rows: db.rows }));
}

if (args[0] === 'app' && args[1] === '--help') {
  console.log('herdr-inbox app v1');
} else if (args[0] === 'app-feed') {
  publish();
} else if (args[0] === 'worker') {
  process.exitCode = 0;
} else if (args[0] === 'open') {
  process.exitCode = 1;
} else {
  const result = db.run(args);
  save();
  publish();
  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);
  process.exitCode = result.code;
}
