// The single-row views: an undelivered answer, an alert, a closed row, and the left-out list.

import { div } from 'gpui-kit';
import { h_flex, v_flex } from 'gpui-base';
import { clickable, fit, footer, windowed } from './view-parts.js';
import { askerLabel } from './view-text.js';
import { age, deliveryWords, knownDelivery, retryLabel, severity } from './words.js';

const seconds = () => Date.now() / 1000;

function heading(t, title, parts) {
  return v_flex().flex_none().gap(6).px(22).pt(18).pb(12)
    .child(div().text_size(18).font_semibold().text_color(t.foreground).child(title))
    .child(h_flex().gap(10).font_family(t.mono).text_size(11.5).text_color(t.muted).children(parts.filter(Boolean).map((p, i) => div().when(i === 0, el => el.text_color(t.accent)).child(p))));
}

function choice(mod, t, id, n, label, on) {
  return h_flex().id(id).gap(8).px(12).py(9).rounded(9).border_1().border_color(on ? t.accent : t.border).bg(on ? t.selected : t.raised)
    .map(clickable(t, on, (_e, cx) => mod.onRow(n - 1, cx)))
    .child(div().w(22).font_family(t.mono).text_size(12).text_color(on ? t.accent : t.muted).child(String(n)))
    .child(div().text_size(13.5).text_color(t.foreground).child(label));
}

export function failedBody(mod, t) {
  const v = mod.session.popup.view;
  const row = mod.session.book.row(`failed:${v.qid}`);
  if (!row) return goneText(t, `Q${v.qid}`, 'no longer pending');
  const d = deliveryWords(row);
  return [
    heading(t, d.warn ? `Answer to Q${v.qid} not delivered` : `Answer to Q${v.qid} is waiting`, [`Q${v.qid}`, askerLabel(mod, row.asker), row.asked_by && `asked by ${row.asked_by}`, `${age(row.answered_at, seconds())} ago`]),
    v_flex().id('failed').flex_1().min_h(0).overflow_y_scrollbar().gap(12).px(22).py(12)
      .child(div().text_size(15).text_color(t.foreground).child(row.question))
      .child(div().text_size(13).text_color(t.muted).child(`Your answer: ${row.answer ?? '(not shown)'}`))
      .child(div().text_size(13).text_color(d.warn ? t.blocked : t.muted).child(d.line))
      .when(row.cut, el => el.child(div().text_size(12).text_color(t.blocked).child('This text was cut to fit the feed.')))
      .when(knownDelivery(row), el => el.child(choice(mod, t, 'retry', 1, retryLabel(row), v.cursor === 0))
        .child(choice(mod, t, 'hide', 2, 'Stop showing it here', v.cursor === 1))),
    footer(t, [...(knownDelivery(row) ? [['1-2', 'pick'], ['⏎', 'do it']] : []), ['g', 'go to agent'], ['esc', 'back']]),
  ];
}

export function alertBody(mod, t) {
  const { popup, book } = mod.session;
  const row = book.row(popup.entry);
  if (!row) return goneText(t, popup.entry, 'closed');
  const local = row.kind === 'alert';
  const color = severity(row.severity) === 'info' ? t.muted : t.blocked;
  return [
    heading(t, row.title, [local ? `A${row.id}` : `${row.machine.label ?? row.machine.id}:A${row.id}`, local ? askerLabel(mod, row.asker) : null, row.asked_by, row.created_at && `${age(row.created_at, seconds())} ago`]),
    v_flex().id('alert').flex_1().min_h(0).overflow_y_scrollbar().gap(12).px(22).py(12)
      .child(div().text_size(12).text_color(color).child(severity(row.severity)))
      .child(div().text_size(14).text_color(t.foreground).child(row.message ?? '')),
    footer(t, local ? [['d', 'acknowledge'], ['g', 'go to agent'], ['esc', 'back']] : [['esc', 'back (read-only here)']]),
  ];
}

function goneText(t, ref, reason) {
  return [v_flex().flex_1().items_center().justify_center().gap(6)
    .child(div().text_size(15).text_color(t.foreground).child(`${ref} was closed (${reason})`))
    .child(div().text_color(t.muted).child('Esc goes back.'))];
}

export function goneBody(mod, t) {
  const { ref, reason } = mod.session.popup.view;
  return goneText(t, ref, reason);
}

export function pickerBody(mod, t) {
  const s = mod.session;
  const { ids } = s.incomplete ?? { ids: [] };
  const rows = ids.map(id => {
    const on = s.picker.cursor === id;
    return h_flex().id(`left-${id}`).gap(8).px(16).py(10).border_t_1().border_color(t.border).when(on, el => el.bg(t.selected))
      .map(clickable(t, on, (_e, cx) => mod.onLeftOut(id, cx))).text_size(13.5).text_color(t.foreground)
      .child(div().w(12).text_color(t.accent).child(on ? '❯' : '')).child(div().child(`Q${id}`));
  });
  return [
    h_flex().flex_none().px(16).pt(14).pb(10).text_size(14).font_semibold().text_color(t.foreground).child('Left out of this list'),
    div().px(16).pb(10).text_size(12).text_color(t.muted).child('These did not fit in the feed. Each opens in full here and can be answered as usual.'),
    v_flex().id('left-out').flex_1().min_h(0).overflow_y_scrollbar().children(windowed(t, rows, ids.indexOf(s.picker.cursor), fit(mod, 170, 40))),
    footer(t, [['⏎', 'open'], ['esc', 'back']]),
  ];
}

