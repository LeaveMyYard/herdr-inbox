// Where a key goes: every key is stamped, then dropped inside the grace window; Tab and Shift+Tab
// switch tabs even under the confirmation; the confirmation takes every other key; the empty view
// closes on any key but the navigation keys; otherwise the view's own keys, then the app's.

import { RELEASE, ROW_ARM, digit, hasPreview, isAlert } from './data.js';
import { confirmed, dismiss, goTo, writeDraft } from './actions.js';
import { back, navigate, showList, allTabs } from './groups.js';
import { questionKey } from './question.js';
import { dropTarget, reloadReview, reviewKey } from './review.js';
import { purgeClosed } from './rows.js';
import { alertKey, failedKey, goneKey } from './views.js';
import { viewQuestion } from './remote.js';

// Where the keys go first. Inside an open box Tab moves the focus off it while it stays open.
export function focusOf(p) {
  if (p.view.kind === 'none') return 'chips';
  return p.view.kind === 'question' && p.view.editing && p.view.box_focused ? 'box' : p.focus;
}

const editing = p => focusOf(p) === 'box';

function canSwitch(p) {
  const tabs = p.mode === 'group' || (p.mode === 'empty' && p.entries.length > 0) || (p.mode === 'remote' && p.held);
  return tabs && !editing(p);
}

function canAct(p, goto) {
  if (editing(p)) return false;
  if (p.mode === 'list') {
    const c = listTarget(p);
    return c !== null && p.entries.includes(c) && !p.closed.has(c);
  }
  if (p.mode === 'group') return p.current !== null && p.current !== 'submit' && !p.gone.has(p.current);
  return (p.mode === 'failed' || p.mode === 'alert') && goto;
}

function switchTab(p, delta) {
  if (p.held) {
    if (delta > 0 && p.now >= p.held_at + RELEASE) p.held = false; // these keys only release a deep link
  } else if (p.mode === 'empty') showList(p, p.entries[0] ?? null);
  else navigate(p, delta, null);
}

// Tab or Shift+Tab where they switch nothing: the focus moves to the next focusable part.
function moveFocus(p, next) {
  const v = p.view;
  const chips = p.mode === 'group' && allTabs(p).length > 1;
  if (v.kind === 'question') {
    const preview = Boolean(viewQuestion(p, v.qid) && hasPreview(viewQuestion(p, v.qid)));
    if (v.editing && v.box_focused) {
      v.box_focused = false;
      p.focus = !next ? 'choices' : preview ? 'preview' : chips ? 'chips' : 'view';
      return;
    }
    const chain = ['view', 'choices', ...(v.editing ? ['box'] : []), ...(preview ? ['preview'] : [])];
    const at = Math.max(chain.indexOf(p.focus), 0);
    const to = chain[(at + (next ? 1 : chain.length - 1)) % chain.length];
    v.box_focused = to === 'box';
    p.focus = v.box_focused ? 'view' : to;
  } else if (v.kind === 'list') {
    p.focus = p.focus === 'view' ? 'choices' : 'view';
  }
}

// -- the list's keys: ↑/↓ with no wrap-around, digits name question rows, Enter opens, Esc closes.

export const listTarget = p => (p.view.kind === 'list' ? p.view.cursor : null);

export const fresh = (p, entry) => p.arrived_at.has(entry) && p.now < p.arrived_at.get(entry) + ROW_ARM;

function listMoved(p) {
  purgeClosed(p, listTarget(p)); // a deliberate move: placeholders drop out now, the target stays
}

function listMove(p, delta) {
  const n = p.entries.length;
  if (n === 0) return;
  const at = p.view.cursor === null ? -1 : p.entries.indexOf(p.view.cursor);
  if (at < 0) p.view.cursor = p.entries[delta > 0 ? 0 : n - 1];
  else p.view.cursor = p.entries[Math.min(Math.max(at + delta, 0), n - 1)];
  listMoved(p);
}

function listKey(p, key, armed) {
  const n = digit(key);
  if (key === 'up' || key === 'k') listMove(p, -1);
  else if (key === 'down' || key === 'j') listMove(p, 1);
  else if (key === 'escape') p.later({ t: 'quit' });
  else if (key === 'enter' && armed) {
    const c = listTarget(p);
    if (c !== null && p.entries.includes(c) && !p.closed.has(c) && !fresh(p, c)) p.later({ t: 'open', entry: c });
  } else if (n !== null && armed) {
    const row = p.entries.filter(e => !isAlert(e))[n - 1];
    if (row !== undefined && !fresh(p, row)) {
      p.view.cursor = row;
      listMoved(p);
    }
  } else return false;
  return true;
}

function viewArmed(p) {
  return p.now >= (p.view.armed_at ?? 0);
}

function viewKey(p, key) {
  const armed = !p.held && viewArmed(p);
  const v = p.view;
  if (v.kind === 'list') return listKey(p, key, armed);
  if (v.kind === 'review' && key === 'x') {
    const qid = dropTarget(v);
    if (qid !== null) {
      writeDraft(p, qid, null);
      reloadReview(v, p.data);
    }
    return true;
  }
  if (v.kind === 'question') {
    const had = [Boolean(v.editing), v.box_focused];
    const taken = questionKey(v, key, armed, p.out);
    if ((Boolean(v.editing) !== had[0] || v.box_focused !== had[1]) && (!v.editing || v.box_focused)) p.focus = 'view';
    return taken;
  }
  if (v.kind === 'review') return reviewKey(v, key, armed, p.out);
  if (v.kind === 'gone') return goneKey(key, p.out);
  if (v.kind === 'failed') return failedKey(v, key, armed, p.out);
  if (v.kind === 'alert') return alertKey(key, armed, p.out);
  return false;
}

export function takeKey(p, key) {
  p.timers.prev_key_at = p.timers.last_key_at;
  p.timers.last_key_at = p.now;
  if (p.timers.grace_until !== null && p.now < p.timers.grace_until) return; // meant for the old view
  const tab = key === 'tab' || key === 'shift+tab';
  if (tab && canSwitch(p)) return switchTab(p, key === 'tab' ? 1 : -1);
  if (p.confirm) {
    const { target } = p.confirm;
    p.confirm = null;
    return confirmed(p, target, key === 'y');
  }
  if (p.mode === 'empty') {
    if (key === 'left' && p.entries.length > 0) return switchTab(p, -1);
    if (key === 'right' && p.entries.length > 0) return switchTab(p, 1);
    p.exit();
    if (key === 'escape') p.quit();
    return;
  }
  if (tab) return moveFocus(p, key === 'tab');
  if (focusOf(p) !== 'chips' && viewKey(p, key)) return;
  if (key === 'left' && canSwitch(p)) switchTab(p, -1);
  else if (key === 'right' && canSwitch(p)) switchTab(p, 1);
  else if (key === 'd' && canAct(p, false)) dismiss(p);
  else if (key === 'g' && canAct(p, true)) goTo(p);
  else if (key === 'escape') back(p);
}
