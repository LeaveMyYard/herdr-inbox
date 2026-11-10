// Drives a Session against the stand-in, as main.js does: keys in, commands out, feeds read.
import { readFileSync } from 'node:fs';
import { Session } from '../../mods/herdr-inbox/session.js';
import { StandIn } from './standin.mjs';

export const fixture = name => JSON.parse(readFileSync(new URL(`../../../tests/fixtures/app-feed/${name}.json`, import.meta.url)));

export const question = (id, over = {}) => ({
  id, header: null, question: 'Question Text 345', options: [{ label: 'Option A', description: null, preview: null }, { label: 'Option B', description: null, preview: null }],
  multi_select: false, context: null, asked_by: 'lane', draft: null, cut: false, ...over,
});

export const asker = (over = {}) => ({ host: 'herdr-tui', pane: 'w1:p1', terminal: null, tab_label: 'Project Alpha', cwd: '/work/project-alpha', ...over });

export const group = (id, ids = [id], over = {}) => ({
  key: `group:${id}`, kind: 'group', group: id, title: 'Question Title 123', created_at: '2026-01-02T10:10:00+00:00', asker: asker(),
  questions: ids.map(q => question(q)), actions: ['answer', 'draft', 'dismiss'], token: 'x', ...over,
});

export const failed = (id, delivery = {}, over = {}) => ({
  key: `failed:${id}`, kind: 'failed', question_id: id, group: id, header: null, question: 'Question Text 345', asked_by: 'lane', answer: 'Answer Text 567',
  answered_at: '2026-01-02T10:21:00+00:00', retries: 0, cut: false, asker: asker(),
  delivery: { state: 'failed', reason: 'Question Text 345', wait: null, attempts: 1, pending_since: null, auto_retry: true, ...delivery },
  actions: ['retry', 'stop-showing'], token: 'x', ...over,
});

export const noOmissions = () => ({ groups: 0, questions: 0, failed: 0, alerts: 0, remote: 0, question_ids: [] });

export const world = (entries, over = {}) => ({
  v: 1, generated_at: 1767348000, ok: true, errors: [], omitted: noOmissions(), new_since: { question_id: 0, alert_id: 0 }, entries, ...over,
});

export class Rig {
  constructor(feed, { link = {}, standin = {}, rows = [] } = {}) {
    this.db = new StandIn(feed, { rows, ...standin });
    this.session = new Session(link);
    this.now = 1767348100000;
    this.ran = [];
    this.exec = args => this.db.run(args);
    this.published();
    this.pump();
  }

  get popup() {
    return this.session.popup;
  }

  // The writer publishes, and the mod reads it.
  published() {
    this.tick(100);
    this.db.clock = this.now / 1000 - 1;
    this.session.read(this.db.published(), this.now);
    this.pump();
  }

  read(text) {
    this.session.read(text, this.now);
    this.pump();
  }

  tick(ms = 700) {
    this.now += ms;
  }

  // Runs what the session asks for. `hold` keeps a verb in flight until `pump` runs again.
  pump() {
    let step = this.session.next(this.now);
    while (step) {
      if (step.go) this.ran.push(step.go);
      const result = step.go ? this.goes ?? {} : this.exec(step.args);
      if (!step.go) this.ran.push(step.args);
      this.tick(5);
      this.session.finish(step, result, this.now);
      step = this.session.next(this.now);
    }
    if (this.session.reload) {
      this.session.reload = false;
      this.published();
    }
  }

  // Each key a moment after the last, so that views have armed.
  press(...keys) {
    for (const key of keys) {
      this.tick();
      this.session.key(key, this.now);
      this.pump();
    }
  }

  verbs(name) {
    return this.ran.filter(a => Array.isArray(a) && a[1] === name);
  }

  get said() {
    return this.session.notices.map(n => n.text).join('\n');
  }
}
