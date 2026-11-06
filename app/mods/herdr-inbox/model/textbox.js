// The inline text box under an option (a note) or under "Type something.": printable keys insert at
// the cursor, ←/→/Home/End move it, Backspace/Delete remove, Enter submits. A box opened on existing
// text starts with all of it selected: typing or deleting replaces it, ← and → go to its ends.

import { printable } from './data.js';

export function textBox(value) {
  const chars = Array.from(value);
  return { chars, cursor: chars.length, selected: chars.length > 0 };
}

export const boxValue = box => box.chars.join('');

const isWord = c => /[\p{Alphabetic}\p{N}_]/u.test(c);
const wordStart = (box, i, lo) => i > lo && isWord(box.chars[i]) && !isWord(box.chars[i - 1]);

function wordBefore(box, at) {
  for (let i = at - 1; i >= 0; i -= 1) if (wordStart(box, i, 0)) return i;
  return 0;
}

function wordAfter(box, at) {
  for (let i = at; i < box.chars.length; i += 1) if (wordStart(box, i, at)) return i;
  return box.chars.length;
}

const NAMED = {
  enter: 'submit', left: 'left', right: 'right', home: 'home', end: 'end', backspace: 'del-left', delete: 'del-right',
  'ctrl+a': 'home', 'ctrl+e': 'end', 'ctrl+d': 'del-right', 'ctrl+left': 'word-left', 'ctrl+right': 'word-right',
  'ctrl+w': 'del-word-left', 'ctrl+backspace': 'del-word-right', 'alt+backspace': 'del-word-right',
  'ctrl+u': 'del-all-left', 'super+backspace': 'del-all-left', 'ctrl+k': 'del-all-right',
};

function erase(box, from, to) {
  box.chars.splice(from, to - from);
  box.cursor = from;
  return 'edited';
}

// Takes a key: 'edited', 'submit', or 'ignored' when it is not a key the box takes.
export function boxKey(box, key) {
  const typed = printable(key);
  const action = typed === null ? NAMED[key] : 'insert';
  if (!action) return 'ignored';
  if (action === 'submit') return 'submit';
  const n = box.chars.length;
  if (box.selected) {
    box.selected = false;
    if (action === 'left' || action === 'right') {
      box.cursor = action === 'left' ? 0 : n;
      return 'edited';
    }
    if (action === 'insert') box.chars = [];
    else if (!['home', 'end', 'word-left', 'word-right'].includes(action)) return erase(box, 0, n);
    box.cursor = Math.min(box.cursor, box.chars.length);
  }
  const at = box.cursor;
  const size = box.chars.length;
  switch (action) {
    case 'insert': box.chars.splice(at, 0, typed); box.cursor += 1; break;
    case 'left': box.cursor = Math.max(at - 1, 0); break;
    case 'right': box.cursor = Math.min(at + 1, size); break;
    case 'home': box.cursor = 0; break;
    case 'end': box.cursor = size; break;
    case 'word-left': box.cursor = wordBefore(box, at); break;
    case 'word-right': box.cursor = wordAfter(box, at); break;
    case 'del-left': return erase(box, Math.max(at - 1, 0), at);
    case 'del-right': return erase(box, at, Math.min(at + 1, size));
    case 'del-word-left': return erase(box, wordBefore(box, at), at);
    case 'del-word-right': return erase(box, at, wordAfter(box, at));
    case 'del-all-left': return erase(box, 0, at);
    case 'del-all-right': return erase(box, at, size);
    default: break;
  }
  return 'edited';
}
