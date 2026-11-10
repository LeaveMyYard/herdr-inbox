// Which keys count. Pure: what the session asks before a key reaches the model.

// A repeat from a held key never sends, opens or confirms: it can only move a cursor or edit an open box.
const MOVES = new Set(['up', 'down', 'k', 'j']);

export function repeatCounts(popup, picking, name) {
  if (picking) return MOVES.has(name);
  if (popup.confirm || popup.mode === 'empty') return false;
  if (popup.view.box_focused) return name !== 'enter' && name !== 'escape';
  return MOVES.has(name);
}

// The Latin letter on the same key, for a layout that gives another script: outside a text box a key
// means its place on the keyboard, as `d` still dismisses on a Cyrillic layout. ЙЦУКЕН, and Ukrainian.
const ROWS = [
  ['йцукенгшщзхъ', 'qwertyuiop[]'], ['фывапролджэ', "asdfghjkl;'"], ['ячсмитьбю', 'zxcvbnm,.'], ['ё', '`'], ['іїєґ', 's]\'u'],
];
const LATIN = new Map(ROWS.flatMap(([from, to]) => Array.from(from).map((c, i) => [c, to[i]])));

export function latinKey(name) {
  const plus = name.lastIndexOf('+');
  const key = plus > 0 ? name.slice(plus + 1) : name;
  const lower = key.toLowerCase();
  if (!LATIN.has(lower)) return name;
  const latin = key === lower ? LATIN.get(lower) : LATIN.get(lower).toUpperCase();
  return plus > 0 ? `${name.slice(0, plus + 1)}${latin}` : latin;
}
