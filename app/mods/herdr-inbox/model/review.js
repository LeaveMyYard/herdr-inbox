// The open group's Submit tab: every open question with its draft, then Submit answers / Cancel.
// Its cursor is a target; when its question goes it parks on nothing.

import { question } from './data.js';

export function reloadReview(v, data) {
  v.items = v.qids.filter(id => question(data, id)?.status === 'open');
  v.drafted = v.items.filter(id => question(data, id)?.draft);
  if (v.cursor !== null && !targets(v).includes(v.cursor)) v.cursor = null;
}

export function reviewView(qids, data, armedAt) {
  const v = { kind: 'review', qids, items: [], drafted: [], cursor: 'submit', armed_at: armedAt };
  reloadReview(v, data);
  return v;
}

export const targets = v => [...v.items.map(id => `item:${id}`), 'submit', 'cancel'];

export const dropTarget = v => (v.cursor?.startsWith('item:') ? Number(v.cursor.slice(5)) : null);

function step(v, delta) {
  const all = targets(v);
  const at = v.cursor === null ? -1 : all.indexOf(v.cursor);
  if (at < 0) v.cursor = delta > 0 ? all[0] : all[all.length - 1];
  else v.cursor = all[(at + delta + all.length) % all.length];
}

function select(v, out) {
  if (v.cursor === null) return;
  if (v.cursor === 'cancel') out.later({ t: 'back' });
  else if (v.cursor !== 'submit') out.later({ t: 'edit', qid: dropTarget(v) });
  else if (v.drafted.length === 0) out.notify('Nothing answered yet', 'warning');
  else out.later({ t: 'submit', qids: [...v.drafted] });
}

// Takes a key; false if the view does not bind it. `x`, dropping the row's draft, is the caller's.
export function reviewKey(v, key, armed, out) {
  if (key === 'up' || key === 'k') step(v, -1);
  else if (key === 'down' || key === 'j') step(v, 1);
  else if (key === 'escape') out.later({ t: 'back' });
  else if (key === 'enter' && armed) select(v, out);
  else if ((key === '1' || key === '2') && armed) {
    v.cursor = key === '1' ? 'submit' : 'cancel';
    select(v, out);
  } else return false;
  return true;
}
