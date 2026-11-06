// Background events. A poll never changes what the next key does: arrivals are appended (armed
// ROW_ARM later), ended rows and tabs become inert placeholders, nothing joins an open group, and the
// view on screen is replaced only when what it shows ended, with the GRACE window.

import { GRACE, idOf, keyOf, kindOf, machineName, tabId } from './data.js';
import { lastOpen, mount, openQids, show } from './groups.js';
import { pollRemote } from './remote.js';
import { reloadReview } from './review.js';
import { currentEntries, entryReason, liveEntries, tabReason } from './rows.js';

export function replaceWithPlaceholder(p, ref, reason) {
  p.timers.grace_until = p.now + GRACE;
  mount(p, { kind: 'gone', ref, reason });
}

function remoteAlertRef(p, key) {
  const at = key.lastIndexOf('|');
  return `${machineName(p.data.remote, at < 0 ? '' : key.slice(0, at))}:A${key.slice(at + 1)}`;
}

// The open group: its closed tabs stay as placeholders; the view on screen is rebuilt only if its own
// question closed.
function pollGroup(p) {
  const open = new Set(p.data.open);
  const isOpen = t => t !== 'submit' && open.has(tabId(t));
  const ended = p.tabs.filter(t => !isOpen(t) && !p.gone.has(t));
  p.tabs = p.tabs.filter(t => isOpen(t) || t === p.current || p.gone.has(t));
  if (p.current !== null && ended.includes(p.current)) {
    const cur = p.current;
    const reason = tabReason(p, cur);
    p.notify(`Q${tabId(cur)} was closed (${reason})`, 'warning');
    p.gone.set(cur, reason);
    p.timers.grace_until = p.now + GRACE;
    return show(p, cur); // the same tab, now the placeholder
  }
  const v = p.view;
  if (v.kind === 'question') {
    v.in_group = !p.single;
    v.last = lastOpen(p, v.qid);
  } else if (v.kind === 'review') {
    v.qids = openQids(p);
    reloadReview(v, p.data);
  }
}

export function poll(p, snapshot) {
  p.data = snapshot;
  if (p.confirm) return; // a confirmation is open: the rows wait for it
  const current = currentEntries(p, null);
  for (const e of p.entries.filter(x => !current.includes(x) && !p.closed.has(x))) p.closed.set(e, entryReason(p, e));
  const arrivals = current.filter(e => !p.entries.includes(e));
  for (const e of arrivals) p.arrived_at.set(e, p.now);
  p.entries.push(...arrivals);
  const ended = p.entry === null ? undefined : p.closed.get(p.entry);
  const placeholder = p.view.kind === 'gone';
  const kind = p.entry === null ? null : kindOf(p.entry);
  if (p.mode === 'failed' && kind === 'failed' && ended !== undefined && !placeholder) {
    replaceWithPlaceholder(p, `Q${idOf(p.entry)}`, ended);
  } else if (p.mode === 'alert' && kind === 'alert' && ended !== undefined && !placeholder) {
    replaceWithPlaceholder(p, `A${idOf(p.entry)}`, ended);
  } else if (p.mode === 'alert' && kind === 'ralert' && ended !== undefined && !placeholder) {
    replaceWithPlaceholder(p, remoteAlertRef(p, keyOf(p.entry)), ended);
  } else if (p.mode === 'remote' && !placeholder) {
    pollRemote(p);
  } else if (p.mode === 'group') {
    pollGroup(p);
  }
}

export function sent(p, done, wanted, snapshot) {
  const refs = done.map(q => `Q${q}`).join(', ');
  if (done.length === 0) p.notify('Nothing sent: those questions are no longer open', 'warning');
  else p.notify(`Sending ${refs} (in the background; see ⏳/⚠ here if it waits)`, 'information');
  if (done.length > 0 && done.length < wanted) {
    p.notify(`${wanted - done.length} answered question(s) had closed meanwhile`, 'warning');
  }
  poll(p, snapshot);
  if ((p.mode === 'list' || p.mode === 'empty') && liveEntries(p).length === 0 && done.length > 0) p.exit();
}

export function focused(p, pane, error) {
  if (error !== null) p.notify(`could not focus ${pane}: ${error}`, 'warning');
  else p.exit(); // the question stays open; discuss it in the agent's chat
}
