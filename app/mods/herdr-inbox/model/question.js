// One question: numbered options, "Type something.", then "Chat about this" (single-select) or
// "Submit" (multi-select); notes under options; a text box for each. Its rows are fixed for the
// view's life (a question's options never change), so here alone a cursor may be an index.

import { digit, makeDetail, strip } from './data.js';
import { boxKey, boxValue, textBox } from './textbox.js';

const numeric = (a, b) => a - b;

export function questionView(q, inGroup, last, armedAt) {
  const labels = q.options.map(o => o.label);
  const draft = q.draft ?? { choices: [], notes: {}, text: null };
  const picked = [...new Set(draft.choices.map(l => labels.indexOf(l)).filter(i => i >= 0))].sort(numeric);
  const notes = new Map();
  for (const [label, note] of Object.entries(draft.notes)) {
    const i = labels.indexOf(label);
    if (i >= 0) notes.set(i, note);
  }
  const rows = [...labels.map((_, i) => `opt:${i}`), 'other', q.multi_select ? 'submit' : 'chat'];
  const cursor = picked.length > 0 ? picked[0] : draft.text !== null ? labels.length : 0;
  return {
    kind: 'question', qid: q.id, in_group: inGroup, last, multi_select: q.multi_select, labels, rows, cursor, picked, notes,
    text: draft.text, editing: null, box_focused: false, armed_at: armedAt,
  };
}

const optionAt = row => (row.startsWith('opt:') ? Number(row.slice(4)) : null);

function openBox(v, editing) {
  v.editing = editing;
  v.box_focused = true;
}

function closeBox(v) {
  v.editing = null;
  v.box_focused = false;
}

const openText = v => openBox(v, { kind: 'text', box: textBox(v.text ?? '') });

function toggle(v) {
  if (!v.multi_select) return;
  const row = v.rows[v.cursor];
  const i = optionAt(row);
  if (i !== null) v.picked = v.picked.includes(i) ? v.picked.filter(x => x !== i) : [...v.picked, i].sort(numeric);
  else if (row === 'other') openText(v);
}

function answer(v, out) {
  const choices = v.picked.map(i => v.labels[i]);
  const notes = [...v.notes].map(([i, note]) => [v.labels[i], note]);
  out.later({ t: 'answered', qid: v.qid, detail: makeDetail(v.text, choices, notes) });
}

function select(v, out) {
  const row = v.rows[v.cursor];
  const i = optionAt(row);
  if (row === 'chat') out.later({ t: 'chat', qid: v.qid });
  else if (row === 'other') openText(v);
  else if (row === 'submit' && v.picked.length === 0 && v.text === null) out.notify('Select at least one option', 'warning');
  else if (row === 'submit') answer(v, out);
  else if (v.multi_select) toggle(v);
  else {
    v.picked = [i];
    v.text = null;
    answer(v, out);
  }
}

function openNote(v) {
  const i = optionAt(v.rows[v.cursor]);
  if (i !== null) openBox(v, { kind: 'note', i, box: textBox(v.notes.get(i) ?? '') });
}

function clear(v, out) {
  v.picked = [];
  v.notes = new Map();
  v.text = null;
  if (v.in_group) out.later({ t: 'answered', qid: v.qid, detail: null });
}

function submitBox(v, out) {
  v.box_focused = false;
  const editing = v.editing;
  v.editing = null;
  if (!editing) return;
  const value = strip(boxValue(editing.box));
  if (editing.kind === 'note') {
    if (value === '') v.notes.delete(editing.i);
    else v.notes.set(editing.i, value);
  } else if (v.multi_select) {
    v.text = value === '' ? null : value;
  } else if (value !== '') {
    v.picked = [];
    v.text = value;
    answer(v, out);
  }
}

// A click on a row: that row and no other, as Enter on it would be.
export function pickRow(v, row, out) {
  if (row < 0 || row >= v.rows.length) return;
  v.cursor = row;
  select(v, out);
}

// Takes a key; false if the view does not bind it (the app may).
export function questionKey(v, key, armed, out) {
  if (v.editing && v.box_focused) {
    const did = boxKey(v.editing.box, key);
    if (did === 'submit') submitBox(v, out);
    else if (did === 'ignored' && key === 'escape') closeBox(v);
    return true; // the box has the focus: nothing else gets it
  }
  const n = digit(key);
  if ((key === 'enter' || key === 'space' || n !== null) && !armed) return false;
  if (key === 'up' || key === 'k') v.cursor = (v.cursor + v.rows.length - 1) % v.rows.length;
  else if (key === 'down' || key === 'j') v.cursor = (v.cursor + 1) % v.rows.length;
  else if (key === 'enter') select(v, out);
  else if (key === 'space') toggle(v);
  else if (key === 'n') openNote(v);
  else if (key === 'x') clear(v, out);
  else if (key === 'escape' && v.editing) closeBox(v);
  else if (key === 'escape') out.later({ t: 'back' });
  else if (n === null) return false;
  else if (n <= v.rows.length && v.rows[n - 1] !== 'submit') v.cursor = n - 1;
  return true;
}
