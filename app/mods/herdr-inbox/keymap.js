// A host key event as the model names keys. The model's key names are its golden's.

import { latinKey } from './input.js';

const NAMED = new Set(['enter', 'escape', 'tab', 'up', 'down', 'left', 'right', 'home', 'end', 'backspace', 'delete', 'space']);

// The model's name for this event, or null for a key on its own (a modifier).
export function keyName(e) {
  const m = e.modifiers ?? {};
  const mods = [m.control && 'ctrl', m.alt && 'alt', m.platform && 'super'].filter(Boolean);
  if (e.key === 'tab') return mods.length === 0 ? (m.shift ? 'shift+tab' : 'tab') : null;
  if (mods.length > 0) return [...mods, e.key].join('+');
  if (NAMED.has(e.key)) return e.key;
  const typed = e.key_char;
  if (typeof typed === 'string' && Array.from(typed).length === 1) return typed === ' ' ? 'space' : typed;
  return Array.from(e.key).length === 1 ? e.key : null;
}

const isAscii = name => /^[\x20-\x7e]$/.test(name);

// What a key means outside a text box: its place on the keyboard. The host's `key` is the Latin letter
// of the key on a Cyrillic layout, where `key_char` is the letter it types, so `d` dismisses on either.
// Where `key` isn't Latin either (macOS may give the layout's letter), the letter's place is looked up.
export function acting(e, typed) {
  if (typed === null || Array.from(typed).length !== 1 || isAscii(typed)) return typed;
  const key = Array.from(e.key).length === 1 ? e.key : typed;
  return latinKey(e.modifiers?.shift ? key.toUpperCase() : key);
}

export const isPaste = name => name === 'ctrl+v' || name === 'super+v';

// Text as keys, for a paste. A line break ends nothing here: it reads as a space.
export function pastedKeys(text) {
  return Array.from(text.replace(/\r?\n/g, ' ')).map(c => (c === ' ' ? 'space' : c)).filter(c => c === 'space' || c >= ' ');
}
