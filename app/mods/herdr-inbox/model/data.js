// What the model reads: a snapshot of the inbox, and the small rules on its rows and names.
// A cursor is always one of these names, never an index, so rows moving under it can't move it.

export const VIEW_ARM = 400; // a view ignores Enter, digits and Space this long after it appears
export const ROW_ARM = 1000; // a row that arrived takes no Enter and no digit this long
export const GRACE = 500; // after a view change the operator did not cause, keys are dropped this long
export const RELEASE = 500; // a held deep link is released by Tab/→ only this long after opening
export const GO_PAUSE = 500; // `g` to an agent at a prompt needs this pause

const AGENT_GONE = 'agent gone: ';

export const isAlert = entry => entry.startsWith('alert:') || entry.startsWith('ralert:');
export const isRemote = entry => /^(remote|routbox|rproblem|ralert):/.test(entry);
export const kindOf = entry => (entry === 'gone' ? 'gone' : entry.slice(0, entry.indexOf(':')));
export const idOf = entry => Number(entry.slice(entry.indexOf(':') + 1));
export const keyOf = entry => entry.slice(entry.indexOf(':') + 1);
export const tabId = tab => Number(tab.slice(2));

export const question = (data, id) => data.questions.find(q => q.id === id);
export const alert = (data, id) => data.alerts.find(a => a.id === id);
export const group = (data, id) => data.questions.filter(q => q.group === id).sort((a, b) => a.id - b.id);
export const openGroup = (data, id) => group(data, id).filter(q => q.status === 'open');
export const agentGone = q => q.delivery_error?.startsWith(AGENT_GONE) ?? false;
export const hasPreview = q => q.options.some(o => Boolean(o.preview));

// Python's str.strip(): Unicode whitespace at both ends.
const SPACE = '[\\t-\\r\\x1c-\\x20\\x85\\xa0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000]+';
const ENDS = new RegExp(`^${SPACE}|${SPACE}$`, 'g');
export const strip = text => text.replace(ENDS, '');

// The answer as it stands: notes only for picked labels, stripped, empty ones dropped; blank text is
// none. Nothing picked and no text is no answer at all. `notes` is pairs, in the order they were written.
export function makeDetail(text, choices, notes) {
  const kept = notes.filter(([label, note]) => choices.includes(label) && strip(note) !== '');
  const typed = text === null ? '' : strip(text);
  if (choices.length === 0 && typed === '') return null;
  return { choices, notes: Object.fromEntries(kept.map(([label, note]) => [label, strip(note)])), text: typed === '' ? null : typed };
}

export const machineName = (remote, id) => remote.machines.find(m => m.machine_id === id)?.label || id;
export const remoteGroup = (remote, key) => remote.groups.find(g => g.key === key && g.questions.length > 0);
export const remoteAlertKey = a => `${a.machine_id}|${a.alert.id}`;
export const remoteAlert = (remote, key) => remote.alerts.find(a => remoteAlertKey(a) === key);
export const problemKey = p => `${p.machine_id}|${p.remote_id}`;
export const remoteRef = q => ({ machine_id: q.machine_id, remote_id: q.question.id, db_id: q.db_id, qkey: q.qkey });

// Why an action on this question would be refused now, or null: it goes under the identity shown.
export function remoteRefusal(remote, ref) {
  const row = remote.mirror.find(m => m.machine_id === ref.machine_id && m.remote_id === ref.remote_id);
  if (!row) return 'that remote question is gone';
  return row.qkey !== ref.qkey || row.db_id !== ref.db_id ? 'that question changed — reopen it' : null;
}

export function markBusy(remote, ref) {
  for (const g of remote.groups) {
    g.questions = g.questions.filter(q => !(q.machine_id === ref.machine_id && q.question.id === ref.remote_id));
  }
  remote.groups = remote.groups.filter(g => g.questions.length > 0);
}

export const emptyRemote = () => ({ machines: [], groups: [], mirror: [], alerts: [], outbox: [], problems: [] });

// A deep copy of plain data, Maps and Sets: what a refusal goes back to.
export function copied(value) {
  if (Array.isArray(value)) return value.map(copied);
  if (value instanceof Map) return new Map([...value].map(([k, v]) => [k, copied(v)]));
  if (value === null || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, copied(v)]));
}
