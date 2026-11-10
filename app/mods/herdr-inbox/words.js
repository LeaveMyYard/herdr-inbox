// What the overlay says. The mod writes its own words from the feed's and never parses `reason`.

const list = ids => ids.map(id => `Q${id}`).join(', ');

export function refusalWords(verb, { code, message, closed }) {
  switch (code) {
    case 'stale':
      return closed ? `This changed: it was ${closed} elsewhere. Nothing was sent. Look again.` : 'This group changed, look again. Nothing was sent.';
    case 'already_sending': return message || 'That answer is being delivered right now.';
    case 'busy': return 'The inbox is busy. Nothing changed: try again. Your input stays.';
    case 'not_migrated': return 'The inbox is being upgraded. Nothing changed: try again in a few seconds.';
    default: return message || `The inbox refused this ${verb}. Nothing changed.`;
  }
}

export const unknownWords = why => `The result is unknown (${why}). It may or may not have been done, and it was not repeated. Check the row after this reload.`;

// What a stale row shows: the row as it was, and as it is now (null: it left the inbox).
export function changeWords(before, after) {
  if (after === null) return 'This row left the inbox while you were on it.';
  if (before?.kind !== 'group') return 'This row changed. Look at it again.';
  const left = before.questions.filter(q => !after.questions.some(x => x.id === q.id)).map(q => q.id);
  if (left.length > 0) return `${list(left)} closed in this group. Look at the rest again.`;
  const drafts = after.questions.filter(q => JSON.stringify(q.draft) !== JSON.stringify(before.questions.find(x => x.id === q.id)?.draft));
  if (drafts.length > 0) return `The stored answer of ${list(drafts.map(q => q.id))} changed elsewhere. Look at it again.`;
  return 'This group changed. Look at it again.';
}

// A `held` result: recorded, and no sender started.
export function heldWords(verb, done) {
  if (verb === 'retry') return `Nothing was sent for ${list(done)}: its agent can't be reached now. The agent can still fetch the answer with check_answers.`;
  return `${list(done)} recorded, but nothing was sent: the agent can't be reached now. The agent can still fetch the answer with check_answers.`;
}

export const queuedWords = done => `Sending ${list(done)} in the background; it is not delivered yet. ⏳/⚠ here says if it waits.`;
export const skippedWords = skipped => `${list(skipped)} had no stored answer and was not sent.`;
export const closedWords = n => `${n} answered question(s) had closed meanwhile and were not sent.`;
export const noneOpenWords = 'Nothing sent: those questions are no longer open.';

export const leftOutWords = counts => `Still open, and not listed here: ${counts.join(', ')}.`;

const WAIT = {
  busy_idle: 'It sends when the agent is idle.',
  requeued: 'Not delivered earlier; it resends when the agent is idle.',
  panel: "The agent's task panel has focus.",
  busy: 'The agent is busy; it sends when the grace ends.',
  blocked: 'The agent is at a prompt or dialog.',
  operator_here: 'It waits for you to leave that pane.',
};

// Where an undelivered answer stands. `pending` that retries by itself is delivery at work, no failure.
export function deliveryWords(row) {
  const d = row.delivery;
  switch (d.state) {
    case 'unattempted': return { warn: !d.auto_retry, line: 'Not delivered yet: nobody has tried.' };
    case 'claimed': return { warn: true, line: 'A process stopped while sending this.' };
    case 'pending': return { warn: !d.auto_retry, line: WAIT[d.wait] ?? d.reason ?? 'It waits.' };
    case 'agent_gone': return { warn: true, line: `The agent is gone${d.reason ? `: ${d.reason}` : ''}. check_answers can still return the answer.` };
    case 'failed': return { warn: true, line: `The last attempt failed${d.reason ? `: ${d.reason}` : ''}.` };
    default: return { warn: true, line: d.reason ?? 'Its state is not known here.' };
  }
}

const DELIVERY = new Set(['unattempted', 'claimed', 'pending', 'agent_gone', 'failed']);

// A word the reader does not know stays visible, with no action on it.
export const knownDelivery = row => DELIVERY.has(row.delivery.state);

// `retry` is "Send now" only on a TUI row that waits for a busy agent: a core row is never forced.
export function retryLabel(row) {
  const now = row.asker.host === 'herdr-tui' && row.delivery.state === 'pending' && (row.delivery.wait === 'busy' || row.delivery.wait === 'busy_idle');
  return now ? 'Send now' : 'Retry delivery';
}

// A time as "4m ago"; `now` in seconds. A time that can't be read says nothing.
export function age(time, now) {
  const at = Date.parse(time) / 1000;
  if (!Number.isFinite(at)) return '';
  const s = Math.max(Math.floor(now - at), 0);
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

const SEVERITY = new Set(['urgent', 'warning', 'info']);
export const severity = word => (SEVERITY.has(word) ? word : 'warning');
export const alertKind = word => (word === 'blocked' ? 'blocked' : 'agent');
