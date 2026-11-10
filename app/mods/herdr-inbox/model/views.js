// The keys of the small views: a closed tab's placeholder, an undelivered answer, an alert.

export function goneKey(key, out) {
  if (key !== 'escape') return false;
  out.later({ t: 'back' });
  return true;
}

const pickFailed = (v, out) => out.later(v.cursor === 0 ? { t: 'retry', qid: v.qid } : { t: 'hide', qid: v.qid });

// A click on a choice: 0 is Retry delivery, 1 is Stop showing it here.
export function failedPick(v, row, out) {
  if (row !== 0 && row !== 1) return;
  v.cursor = row;
  pickFailed(v, out);
}

// ↑/↓, Enter, 1/2 (armed), Esc. Cursor 0 is Retry delivery, 1 is Stop showing it here.
export function failedKey(v, key, armed, out) {
  if (key === 'up' || key === 'down') v.cursor = (v.cursor + 1) % 2;
  else if (key === 'escape') out.later({ t: 'back' });
  else if (key === 'enter' && armed) pickFailed(v, out);
  else if ((key === '1' || key === '2') && armed) {
    v.cursor = key === '2' ? 1 : 0;
    pickFailed(v, out);
  } else return false;
  return true;
}

// `d` (armed) acknowledges; Esc goes back.
export function alertKey(key, armed, out) {
  if (key === 'd' && armed) out.later({ t: 'ack' });
  else if (key === 'escape') out.later({ t: 'back' });
  else return false;
  return true;
}
