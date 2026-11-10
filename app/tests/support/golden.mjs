// The transition golden's JSON, both ways: snapshots and events in, the model's state out, in the
// golden's form. The Rust model has the same adapter (crates/inbox-core/tests/popup/json.rs).
import { readFileSync } from 'node:fs';
import { ROW_ARM } from '../../mods/herdr-inbox/model/data.js';
import { focusOf } from '../../mods/herdr-inbox/model/keys.js';
import { Popup } from '../../mods/herdr-inbox/model/popup.js';
import { boxValue } from '../../mods/herdr-inbox/model/textbox.js';

const fixture = name => JSON.parse(readFileSync(new URL(`../../../tests/fixtures/${name}`, import.meta.url)));

export const golden = fixture('popup-transitions.json');
export const deviations = fixture('popup-deviations.json');

function question(q) {
  const options = q.options.map(label => ({ label, description: null, preview: null }));
  if (q.has_preview) options[0].preview = 'preview'; // the golden says only that some option has one
  return { draft: null, delivery_error: null, delivered: false, ...q, options };
}

function remote(r) {
  const machines = (r.machines ?? []).map(m => ({ machine_id: m.machine_id, label: m.label ?? null, offline: null }));
  const groups = (r.groups ?? []).map(g => ({
    key: g.key,
    machine_id: g.machine_id,
    questions: g.questions.map(q => ({ machine_id: g.machine_id, db_id: q.db_id, qkey: q.qkey, question: question(q) })),
  }));
  const alerts = (r.alerts ?? []).map(a => ({ machine_id: a.machine_id, db_id: a.db_id, akey: a.akey, alert: a }));
  return { machines, groups, mirror: r.mirror ?? [], alerts, outbox: r.outbox ?? [], problems: r.problems ?? [] };
}

export function snapshot(v) {
  return { ...v, questions: v.questions.map(question), ...(v.remote ? { remote: remote(v.remote) } : { remote: remote({}) }) };
}

export function open(v) {
  return { focus_pane: v.focus_pane ?? null, snapshot: snapshot(v.snapshot), deeplink: v.deeplink ?? null };
}

export function event(v) {
  if ('poll' in v) return { poll: snapshot(v.poll) };
  if ('data' in v) return { data: snapshot(v.data) };
  if ('sent' in v) return { sent: { ...v.sent, snapshot: snapshot(v.sent.snapshot) } };
  if ('focused' in v) return { focused: { pane: v.focused.pane, error: v.focused.error ?? null } };
  if ('key' in v) return v;
  throw new Error(`unknown event ${JSON.stringify(v)}`);
}

function view(p) {
  const v = p.view;
  if (v.kind === 'none' || v.kind === 'failed' || v.kind === 'gone') {
    return v.kind === 'none' ? { kind: 'none' } : v.kind === 'gone' ? { kind: 'gone', ref: v.ref, reason: v.reason }
      : { kind: 'failed', qid: v.qid, cursor: v.cursor, armed_at: v.armed_at };
  }
  if (v.kind === 'list') return { kind: 'list', cursor: v.cursor, armed_at: v.armed_at };
  if (v.kind === 'alert') return { kind: 'alert', alert: v.alert, label: v.label, armed_at: v.armed_at };
  if (v.kind === 'review') return { kind: 'review', items: v.items, cursor: v.cursor, armed_at: v.armed_at };
  const e = v.editing;
  return {
    kind: 'question', qid: v.qid, in_group: v.in_group, last: v.last, cursor: v.rows[v.cursor], picked: v.picked,
    notes: Object.fromEntries(v.notes), text: v.text,
    editing: e === null ? null : e.kind === 'note' ? `note:${e.i}` : 'text',
    input: e === null ? null : { value: boxValue(e.box), cursor: e.box.cursor }, armed_at: v.armed_at,
  };
}

export function state(p) {
  if (!p.running) return { running: false };
  const fresh = p.entries.filter(e => p.arrived_at.has(e)).map(e => [e, p.arrived_at.get(e) + ROW_ARM]);
  return {
    running: true, focus: focusOf(p), mode: p.mode, entries: p.entries, closed: Object.fromEntries(p.closed),
    fresh_until: Object.fromEntries(fresh), list_cursor: p.list_cursor, direct: p.direct, held: p.held, entry: p.entry, tabs: p.tabs,
    current: p.current, gone: Object.fromEntries(p.gone), single: p.single, ...p.timers, view: view(p), confirm: p.confirm?.what ?? null,
  };
}

// The scenario as this model must play it: Python's, up to a deviation (the Rust model's too).
export function withDeviation(scenario) {
  const d = deviations.transitions.find(x => x.scenario === scenario.name);
  return d ? { ...scenario, steps: [...scenario.steps.slice(0, d.from_step), ...d.steps] } : scenario;
}

// Every step's effects and state, in the golden's form, as [effects, state].
export function played(scenario, make = (o, at) => new Popup(o, at)) {
  const [first, ...rest] = scenario.steps;
  const popup = make(open(first.event.open), first.at);
  const out = [[popup.first, JSON.parse(JSON.stringify(state(popup)))]];
  for (const step of rest) {
    const effects = popup.handle(event(step.event), step.at);
    out.push([effects, JSON.parse(JSON.stringify(state(popup)))]);
  }
  return out;
}
