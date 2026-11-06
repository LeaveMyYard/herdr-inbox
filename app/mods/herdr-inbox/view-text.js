// The words on a list row, from the book. Unknown vocabulary keeps the row visible with its fallback.

import { goneBatchLive } from './model/rows.js';
import { alertKind, age, deliveryWords, severity } from './words.js';

export function askerLabel(mod, asker) {
  if (asker.host === 'herdr-core') {
    const a = mod.agentsNow().find(x => x.address === asker.terminal);
    return a ? `${a.space_name} · tab ${a.tab_index}` : asker.cwd?.split('/').pop() ?? asker.terminal ?? 'unknown';
  }
  return asker.tab_label ?? asker.cwd?.split('/').pop() ?? 'unknown';
}

// The rows to draw so that the one under the cursor is always on screen: { from, to } of `total`.
// Nothing may be picked out of sight, so the rows are windowed here and not left to a scroll position.
export function windowOf(at, total, size) {
  if (total <= size) return { from: 0, to: total };
  const from = Math.min(Math.max(at - Math.floor(size / 2), 0), total - size);
  return { from, to: from + size };
}

const dot = parts => parts.filter(Boolean).join(' · ');
const questions = n => `${n} ${n === 1 ? 'question' : 'questions'}`;

export function describe(mod, entry, now) {
  const { book, popup } = mod.session;
  const row = book.shown(entry);
  if (entry === 'gone') {
    const n = goneBatchLive(popup).length;
    return { mark: '⚠', warn: true, title: `${n} answers whose agents are gone`, meta: '', sub: 'd hides them; check_answers still has them' };
  }
  if (!row) return { title: entry, meta: '', sub: '' };
  switch (row.kind) {
    case 'group': {
      const first = row.questions[0];
      const left = row.left_out ? 'left out of the feed' : '';
      return {
        title: row.title ?? first.header ?? first.question.split('\n')[0],
        meta: dot([questions(row.questions.length), age(row.created_at, now)]),
        sub: dot([askerLabel(mod, row.asker), first.asked_by, row.questions.map(q => q.header).filter(Boolean).join(', '), left]),
      };
    }
    case 'failed': {
      const d = deliveryWords(row);
      return {
        mark: d.warn ? '⚠' : '⏳', warn: d.warn, title: `Answer to Q${row.question_id} not delivered`,
        meta: dot([row.delivery.state === 'agent_gone' ? 'agent gone' : null, age(row.answered_at, now)]),
        sub: dot([askerLabel(mod, row.asker), row.asked_by, d.line]),
      };
    }
    case 'alert':
      return {
        mark: alertKind(row.alert_kind) === 'blocked' ? '⏸' : '!', warn: severity(row.severity) !== 'info', title: row.title,
        meta: age(row.created_at, now), sub: dot([askerLabel(mod, row.asker), row.asked_by, row.message?.split('\n')[0]]),
      };
    case 'remote_group':
      return {
        title: row.title ?? row.questions[0].question.split('\n')[0], meta: dot([questions(row.questions.length), 'read-only']),
        sub: dot([row.machine.label ?? row.machine.id, row.tab_label, row.machine.offline]),
      };
    case 'remote_alert':
      return { mark: '!', warn: true, title: row.title, meta: 'read-only', sub: dot([row.machine.label ?? row.machine.id, row.message?.split('\n')[0]]) };
    case 'remote_outbox':
      return {
        mark: row.state === 'queued' ? '⏳' : '⚠', warn: row.state !== 'queued', title: `${row.action} for Q${row.remote_id}`, meta: row.state,
        sub: dot([row.machine.label ?? row.machine.id, row.result]),
      };
    case 'remote_problem':
      return { mark: '⚠', warn: true, title: `Answer to Q${row.remote_id} not delivered`, meta: 'read-only', sub: dot([row.machine.label ?? row.machine.id, row.why]) };
    default:
      return { title: entry, meta: '', sub: '' };
  }
}
