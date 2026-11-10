// The group list's rows: which entries the snapshot holds and in what order, the gone-agents summary
// row, and why a row or a tab closed.

import { agentGone, alert, group, idOf, kindOf, problemKey, question, remoteAlertKey, tabId } from './data.js';

// Alerts first (a remote machine's after the local ones), then groups (the focused pane's first, then
// oldest first), then ⚠ answers, then the remote machines' rows.
export function currentEntries(p, focus) {
  const r = p.data.remote;
  return [
    ...p.data.open_alerts.map(a => `alert:${a}`),
    ...r.alerts.map(a => `ralert:${remoteAlertKey(a)}`),
    ...collapseGone(p, entryOrder(p, focus)),
    ...r.groups.map(g => `remote:${g.key}`),
    ...r.outbox.map(o => `routbox:${o.request_id}`),
    ...r.problems.map(x => `rproblem:${problemKey(x)}`),
  ];
}

function entryOrder(p, focus) {
  const firsts = [];
  for (const id of [...p.data.open].sort((a, b) => a - b)) {
    const q = question(p.data, id);
    if (q && !firsts.some(f => f.group === q.group)) firsts.push({ group: q.group, first: q.id, pane: q.pane_id });
  }
  firsts.sort((a, b) => (a.pane !== focus) - (b.pane !== focus) || a.first - b.first);
  return [...firsts.map(f => `group:${f.group}`), ...p.data.failed.map(id => `failed:${id}`)];
}

// Two or more gone-agent answers when the popup opens become ONE summary row, at the end. The set is
// fixed then, so later ones keep rows of their own.
function collapseGone(p, entries) {
  const failedId = e => (kindOf(e) === 'failed' ? idOf(e) : null);
  const gone = entries.map(failedId).filter(id => id !== null && question(p.data, id) && agentGone(question(p.data, id)));
  p.gone_batch ??= gone.length >= 2 ? gone : [];
  const out = entries.filter(e => !p.gone_batch.includes(failedId(e)));
  return gone.some(id => p.gone_batch.includes(id)) ? [...out, 'gone'] : out;
}

// The summary row's answers still gone and shown.
export function goneBatchLive(p) {
  return (p.gone_batch ?? []).filter(id => p.data.failed.includes(id) && question(p.data, id) && agentGone(question(p.data, id)));
}

export const liveEntries = p => p.entries.filter(e => !p.closed.has(e));

// The operator moved: placeholders drop out, except the one under the cursor.
export function purgeClosed(p, keep) {
  p.entries = p.entries.filter(e => !p.closed.has(e) || e === keep);
  p.closed = new Map([...p.closed].filter(([e]) => e === keep));
}

const REMOTE_REASON = { ralert: 'closed', remote: 'answered or closed', routbox: 'sent', rproblem: 'delivered or cleared' };
const ALERT_REASON = { acked: 'acknowledged', withdrawn: 'withdrawn by the agent', cleared: 'no longer waiting' };

export function entryReason(p, entry) {
  const kind = kindOf(entry);
  if (REMOTE_REASON[kind]) return REMOTE_REASON[kind];
  if (kind === 'alert') return ALERT_REASON[alert(p.data, idOf(entry))?.status] ?? 'closed';
  if (kind === 'failed') return question(p.data, idOf(entry))?.delivered ? 'delivered' : 'no longer pending';
  if (kind === 'gone') return 'hidden or delivered';
  const reasons = new Set(group(p.data, idOf(entry)).map(q => tabReason(p, `q:${q.id}`)));
  return reasons.size === 1 ? [...reasons][0] : 'closed';
}

// Why a question tab closed.
export function tabReason(p, tab) {
  if (tab === 'submit') return 'closed';
  const status = question(p.data, tabId(tab))?.status;
  if (status === undefined) return 'removed';
  return status === 'withdrawn' ? 'withdrawn by the agent' : status;
}
