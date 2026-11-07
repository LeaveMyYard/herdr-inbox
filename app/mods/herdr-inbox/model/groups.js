// Moving between views on the operator's actions: opening a list row, the open group's tabs, back to
// the list, the empty view. A view mounted here is armed VIEW_ARM later.

import { VIEW_ARM, alert, idOf, isAlert, kindOf, openGroup, question, tabId } from './data.js';
import { questionView } from './question.js';
import { openRemote } from './remote.js';
import { reviewView } from './review.js';
import { liveEntries } from './rows.js';

export function mount(p, view) {
  p.timers.view_at = p.now;
  p.view = view;
  p.focus = 'view';
}

export const armedAt = p => p.now + VIEW_ARM;

export function openEntry(p, entry, start) {
  p.list_cursor = entry;
  const kind = kindOf(entry);
  if (kind === 'alert') {
    const id = idOf(entry);
    if (alert(p.data, id)?.status !== 'open') return showList(p, entry);
    Object.assign(p, { mode: 'alert', entry, tabs: [], current: null });
    mount(p, { kind: 'alert', alert: id, label: null, armed_at: armedAt(p) });
  } else if (kind === 'gone') {
    p.notify('Those agents are gone: d hides all these answers (check_answers still has them)', 'information');
  } else if (kind === 'failed') {
    Object.assign(p, { mode: 'failed', entry });
    mount(p, { kind: 'failed', qid: idOf(entry), cursor: 0, armed_at: armedAt(p) });
  } else if (kind === 'group') {
    const qs = openGroup(p.data, idOf(entry));
    if (qs.length === 0) return showList(p, entry);
    Object.assign(p, { mode: 'group', entry, tabs: qs.map(q => `q:${q.id}`), gone: new Map() });
    p.single = qs.length === 1; // decided here, on the operator's move
    const blank = (qs.find(q => !q.draft) ?? qs[0]).id;
    show(p, `q:${qs.some(q => q.id === start) ? start : blank}`);
  } else {
    openRemote(p, entry);
  }
}

// The open group's questions still open (no placeholders).
export const openQids = p => p.tabs.filter(t => !p.gone.has(t) && t !== 'submit').map(tabId);

// The tabs, with the Submit tab unless a lone question or nothing is open.
export function allTabs(p) {
  return !p.single && openQids(p).length > 0 ? [...p.tabs, 'submit'] : [...p.tabs];
}

export const lastOpen = (p, qid) => p.single || openQids(p).at(-1) === qid;

export function show(p, tab) {
  p.current = tab;
  if (tab === null) return backToList(p, false);
  if (tab === 'submit') return mount(p, reviewView(openQids(p), p.data, armedAt(p)));
  const id = tabId(tab);
  const q = question(p.data, id);
  if (p.gone.has(tab)) mount(p, { kind: 'gone', ref: `Q${id}`, reason: p.gone.get(tab) });
  else if (q) mount(p, questionView(q, !p.single, lastOpen(p, id), armedAt(p)));
  else mount(p, { kind: 'gone', ref: `Q${id}`, reason: 'removed' });
}

// Moves between the open group's tabs, by `delta` or to a tab; placeholders drop out on this move.
export function navigate(p, delta, to) {
  const tabs = allTabs(p);
  let target = to;
  if (to === null && tabs.length > 0) {
    const at = Math.max(tabs.indexOf(p.current), 0);
    target = tabs[(((at + delta) % tabs.length) + tabs.length) % tabs.length];
  }
  p.tabs = p.tabs.filter(t => !p.gone.has(t) || t === target);
  p.gone = new Map([...p.gone].filter(([t]) => t === target));
  const now = allTabs(p);
  show(p, now.includes(target) ? target : now[0] ?? null);
}

export function showList(p, cursor) {
  Object.assign(p, { mode: 'list', entry: null, direct: false, tabs: [], current: null, gone: new Map() });
  if (p.entries.length === 0) return showEmpty(p);
  const wanted = cursor ?? p.list_cursor;
  mount(p, { kind: 'list', cursor: p.entries.includes(wanted) ? wanted : null, armed_at: armedAt(p) });
}

export function showEmpty(p) {
  p.mode = 'empty';
  mount(p, { kind: 'none' });
}

// After a send, hide or cancel: the list, the cursor still on the entry just handled; close if nothing
// is left. Open alerts keep the popup open only after an alert action.
export function backToList(p, fromAlert) {
  p.direct = false;
  if (!liveEntries(p).some(e => fromAlert || !isAlert(e))) return p.later({ t: 'quit' });
  showList(p, p.list_cursor);
}

// Esc: back to the list, or close (from the list, or when it opened straight into the only group).
export function back(p) {
  if (['group', 'failed', 'remote', 'alert'].includes(p.mode) && !p.direct) return showList(p, p.list_cursor);
  p.quit();
}
