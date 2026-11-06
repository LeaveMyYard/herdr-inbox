// The parts every view is built from.

import { div } from 'gpui-kit';
import { h_flex } from 'gpui-base';
import { windowOf } from './view-text.js';

export function keys(t, pairs) {
  return pairs.map(([key, what]) => h_flex().gap(4).child(div().text_color(t.accent).child(key)).child(div().child(what)));
}

export function footer(t, pairs, buttons = []) {
  return h_flex().flex_none().items_center().gap(14).px(22).py(12).border_t_1().border_color(t.border).text_size(11.5).text_color(t.muted)
    .child(h_flex().gap(14).font_family(t.mono).children(keys(t, pairs))).child(div().flex_1()).children(buttons);
}

// Rows with their window, and how many are out of sight on each side.
export function windowed(t, rows, at, size) {
  const { from, to } = windowOf(Math.max(at, 0), rows.length, size);
  const more = (id, n, mark) => (n > 0 ? [div().id(id).px(16).py(2).font_family(t.mono).text_size(11.5).text_color(t.muted).child(`${mark} ${n} more`)] : []);
  return [...more('above', from, '↑'), ...rows.slice(from, to), ...more('below', rows.length - to, '↓')];
}

// How many rows of this height fit in what is left of the overlay.
export const fit = (mod, used, row) => Math.max(Math.floor(((mod.size?.height ?? 620) - used - (mod.below ?? 0)) / row), 2);

export function button(t, id, label, primary, click) {
  return h_flex().id(id).h(30).px(12).items_center().gap(8).rounded(7).border_1().cursor_pointer().text_size(12.5)
    .border_color(primary ? t.solid : t.input).bg(primary ? t.solid : t.raised)
    .text_color(primary ? t.solid_foreground : t.foreground).on_click(click).child(label);
}
