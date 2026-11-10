// A stand-in for `herdr-inbox app <verb> --json`, until B4 builds the real one. It follows
// docs/protocol/app-commands-v1.md, and fixtures.test.mjs holds it to every case in
// tests/fixtures/app-commands/commands.json. It is a test double: passing against it proves nothing
// about the Rust verbs.
import { createHash } from 'node:crypto';

const TOKEN_FIELDS = {
  group: ['key', 'group', 'title', 'created_at', 'asker', 'questions'],
  failed: ['key', 'question_id', 'group', 'header', 'question', 'asked_by', 'answer', 'answered_at', 'retries', 'asker'],
  alert: ['key', 'id', 'alert_kind', 'severity', 'title', 'message', 'asked_by', 'created_at', 'asker'],
};

const pick = (object, keys) => Object.fromEntries(keys.map(k => [k, object[k]]));
const ASKER = ['host', 'pane', 'terminal', 'tab_label', 'cwd'];
const DRAFT = ['choices', 'notes', 'text'];
const OPTION = ['label', 'description', 'preview'];
const QUESTION = ['id', 'header', 'question', 'options', 'multi_select', 'context', 'asked_by', 'draft'];

function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const keys = Object.keys(value).sort();
    return `{${keys.map(k => `${JSON.stringify(k)}:${canonical(value[k])}`).join(',')}}`;
  }
  return JSON.stringify(value);
}

export function tokenOf(entry) {
  const fields = pick(entry, TOKEN_FIELDS[entry.kind]);
  fields.asker = pick(fields.asker, ASKER);
  if (fields.questions) {
    fields.questions = fields.questions.map(q => ({
      ...pick(q, QUESTION),
      options: q.options.map(o => pick(o, OPTION)),
      draft: q.draft ? pick(q.draft, DRAFT) : null,
    }));
  }
  return createHash('sha256').update(canonical(fields)).digest('hex').slice(0, 32);
}

const ID = /^[1-9][0-9]{0,15}$/;
const isId = text => ID.test(text) && Number(text) <= 9007199254740991;
const ENTRY = /^(group|failed|alert):(.+)$/;
const VERBS = new Set(['answer', 'draft', 'dismiss', 'retry', 'stop-showing', 'ack', 'show']);
const STALE = 'This group changed; look again.';

const usage = message => ({ code: 2, stdout: '', stderr: `usage: ${message}\n` });

function parseArgs(args) {
  const out = { questions: [], json: false, clear: false };
  for (let i = 0; i < args.length; i += 1) {
    const a = args[i];
    if (a === '--json') out.json = true;
    else if (a === '--clear') out.clear = true;
    else if (['--entry', '--token', '--detail', '--question'].includes(a)) {
      i += 1;
      if (i >= args.length) return null;
      if (a === '--question') out.questions.push(args[i]);
      else if (a.slice(2) in out) return null;
      else out[a.slice(2)] = args[i];
    } else return null;
  }
  return out;
}

export class StandIn {
  // `feed` is what the database holds. `rows` are the rows left out of it, for `show`.
  constructor(feed, { server = '/run/herdr-alpha.sock', host = 'up', session = true, rows = [], b3 = false } = {}) {
    this.feed = structuredClone(feed);
    this.rows = structuredClone(rows);
    Object.assign(this, { server, host, session, b3, claims: new Set(), closedRows: new Map(), lock: null, calls: [], clock: feed.generated_at });
  }

  find(key) {
    return this.feed.entries.find(e => e.key === key) ?? this.rows.find(e => e.key === key);
  }

  remove(key, word) {
    this.feed.entries = this.feed.entries.filter(e => e.key !== key);
    this.rows = this.rows.filter(e => e.key !== key);
    if (word) this.closedRows.set(key, word);
  }

  retoken(entry) {
    if (TOKEN_FIELDS[entry.kind]) entry.token = tokenOf(entry);
    return entry;
  }

  // The feed the writer would publish now.
  published() {
    this.clock += 1;
    const entries = this.feed.entries.map(e => this.retoken(e));
    return JSON.stringify({ ...this.feed, entries, generated_at: this.clock });
  }

  refused(verb, entry, code, message, closed = null) {
    return { code: 1, stdout: JSON.stringify({ v: 1, ok: false, verb, entry, code, message, closed }), stderr: '' };
  }

  done(verb, entry, token, done, skipped = [], delivery = null) {
    return { code: 0, stdout: JSON.stringify({ v: 1, ok: true, verb, entry, token, done, skipped, delivery }), stderr: '' };
  }

  run(argv) {
    this.calls.push(argv);
    if (argv[0] !== 'app' || !VERBS.has(argv[1])) return usage('unknown verb');
    const [, verb, ...rest] = argv;
    const a = parseArgs(rest);
    if (a === null) return usage('bad arguments');
    if (verb === 'show') return this.show(a);
    if (!a.json) return usage('--json is required');
    const m = ENTRY.exec(a.entry ?? '');
    if (m === null ? !a.entry?.startsWith('remote:') : !isId(m[2])) return usage('bad --entry');
    if (!a.questions.every(isId)) return usage('bad --question');
    let detail = null;
    if (a.detail !== undefined) {
      try {
        detail = JSON.parse(a.detail);
      } catch {
        return usage('--detail is no JSON');
      }
      if (typeof detail !== 'object' || detail === null || Buffer.byteLength(a.detail) > 65536) return usage('bad --detail');
      if (Object.keys(detail).some(k => !['choices', 'notes', 'text'].includes(k))) return usage('bad --detail');
    }
    if (this.lock === 'busy') return this.refused(verb, a.entry, 'busy', 'The inbox database is busy; try again in a moment');
    if (this.lock === 'not_migrated') return this.refused(verb, a.entry, 'not_migrated', 'The inbox is being upgraded; try again in a few seconds');
    if (this.lock === 'error') return this.refused(verb, a.entry, 'error', 'The inbox database failed: unable to open database file');
    return this.act(verb, a, detail);
  }

  act(verb, a, detail) {
    const { entry: key, token } = a;
    if (key.startsWith('remote:')) return this.refused(verb, key, 'read_only', 'Answer this in the herdr popup');
    const row = this.find(key);
    if (!row) return this.refused(verb, key, 'stale', STALE, this.closedRows.get(key) ?? null);
    if (!row.actions.includes(verb)) return this.refused(verb, key, 'read_only', 'This row does not take that action');
    if (verb === 'retry' && this.claims.has(row.question_id)) {
      return this.refused(verb, key, 'already_sending', `Q${row.question_id} is already being delivered`);
    }
    if (tokenOf(row) !== token) return this.refused(verb, key, 'stale', STALE);
    if (row.kind === 'group') {
      for (const q of a.questions) {
        if (!row.questions.some(x => x.id === Number(q))) return this.refused(verb, key, 'invalid', `Q${q} is not an open question of this group`);
      }
    }
    return this[verb === 'stop-showing' ? 'stopShowing' : verb](row, a, detail);
  }

  // The answer as the popup would store it, or a refusal's message.
  normalised(q, detail) {
    const labels = q.options.map(o => o.label);
    const choices = detail.choices ?? [];
    const bad = choices.find(c => !labels.includes(c));
    if (bad !== undefined) return { error: `Q${q.id} has no such option` };
    if (new Set(choices).size !== choices.length) return { error: `Q${q.id} has a choice twice` };
    if (!q.multi_select && choices.length > 1) return { error: `Q${q.id} takes one choice` };
    const ordered = labels.filter(l => choices.includes(l));
    const notes = Object.fromEntries(Object.entries(detail.notes ?? {}).filter(([l, n]) => ordered.includes(l) && n.trim() !== '').map(([l, n]) => [l, n.trim()]));
    const text = detail.text?.trim() ? detail.text.trim() : null;
    return ordered.length === 0 && text === null ? { none: true } : { detail: { choices: ordered, notes, text } };
  }

  delivery(row, done) {
    if (done.length === 0) return null;
    if (row.asker.host === 'herdr-core' && !this.b3) return 'held'; // before B3 no core row is delivered
    if (this.host === 'gone' || this.server === null && row.asker.host !== 'herdr-tui') return 'held';
    if (row.asker.host === 'herdr-tui' && this.server === null && !this.session) return 'held';
    return 'queued';
  }

  close(row, ids) {
    row.questions = row.questions.filter(q => !ids.includes(q.id));
    if (row.questions.length === 0) {
      this.remove(row.key, 'answered');
      return null;
    }
    return tokenOf(row);
  }

  answer(row, a, detail) {
    const ids = a.questions.map(Number);
    if (detail !== null) {
      if (row.questions.length > 1) return this.refused('answer', row.key, 'invalid', 'This group has several open questions: use draft, then answer');
      const q = row.questions.find(x => x.id === ids[0]);
      const n = this.normalised(q, detail);
      if (n.error) return this.refused('answer', row.key, 'invalid', n.error);
      if (n.none) return this.refused('answer', row.key, 'invalid', `Q${q.id} has no answer`);
      q.draft = n.detail;
    }
    const done = ids.filter(id => row.questions.find(q => q.id === id)?.draft);
    const skipped = ids.filter(id => !done.includes(id));
    const delivery = this.delivery(row, done);
    this.delivered = [...(this.delivered ?? []), ...done.map(id => ({ id, detail: row.questions.find(q => q.id === id).draft }))];
    return this.done('answer', row.key, done.length > 0 ? this.close(row, done) : tokenOf(row), done, skipped, delivery);
  }

  draft(row, a, detail) {
    const q = row.questions.find(x => x.id === Number(a.questions[0]));
    if (a.clear) q.draft = null;
    else {
      const n = this.normalised(q, detail);
      if (n.error) return this.refused('draft', row.key, 'invalid', n.error);
      q.draft = n.none ? null : n.detail;
    }
    return this.done('draft', row.key, tokenOf(row), [q.id]);
  }

  dismiss(row, a) {
    const ids = a.questions.length > 0 ? a.questions.map(Number) : row.questions.map(q => q.id);
    row.questions = row.questions.filter(q => !ids.includes(q.id));
    if (row.questions.length > 0) return this.done('dismiss', row.key, tokenOf(row), ids);
    this.remove(row.key, 'dismissed');
    return this.done('dismiss', row.key, null, ids);
  }

  retry(row) {
    row.retries += 1;
    const delivery = this.delivery(row, [row.question_id]);
    return this.done('retry', row.key, tokenOf(row), [row.question_id], [], delivery);
  }

  stopShowing(row) {
    this.remove(row.key, 'hidden');
    return this.done('stop-showing', row.key, null, [row.question_id]);
  }

  ack(row) {
    this.remove(row.key, 'acked');
    return this.done('ack', row.key, null, [row.id]);
  }

  show(a) {
    const [id] = a.questions;
    if (a.questions.length !== 1 || !isId(id)) return usage('show takes one --question');
    const has = e => e.kind === 'group' && e.questions.some(q => q.id === Number(id));
    const row = this.feed.entries.find(has) ?? this.rows.find(has);
    if (!row) {
      const [key, word] = [...this.closedRows].find(([k]) => k === `group:${id}`) ?? [`group:${id}`, null];
      return this.refused('show', key, 'stale', STALE, word);
    }
    return { code: 0, stdout: JSON.stringify({ v: 1, ok: true, verb: 'show', entry: row.key, row: this.full?.get(row.key) ?? row }), stderr: '' };
  }
}
