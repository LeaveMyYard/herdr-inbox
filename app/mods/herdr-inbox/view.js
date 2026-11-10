// What the overlay shows: the shell around every state, and the group list. Nothing here decides.

import { div } from 'gpui-kit';
import { h_flex, v_flex } from 'gpui-base';
import { fresh } from './model/keys.js';
import { alertBody, failedBody, goneBody, pickerBody } from './view-rows.js';
import { groupBody } from './view-question.js';
import { button, fit, footer, keys, windowed } from './view-parts.js';
import { describe } from './view-text.js';

// Every line the operator must not miss: stale or unreadable feeds, left-out rows, what just happened.
function banners(mod, t) {
  const s = mod.session;
  const lines = [];
  const line = (id, text, color = t.blocked) => lines.push(div().id(id).px(22).py(6).text_size(12).text_color(color).child(text));
  if (s.stale) line('b-stale', s.problem ? `The inbox feed can't be read (${s.problem}): this is the last good copy. It may be out of date.` : 'The inbox feed is out of date: this may not be the inbox as it is now.');
  for (const [i, e] of s.errors.entries()) line(`b-err-${i}`, e);
  const inc = s.incomplete;
  if (inc?.counts.length > 0) {
    const how = inc.ids.length > 0 ? ' Press o to open them.' : '';
    line('b-left', `Left out of this list: ${inc.counts.join(', ')}.${how}`);
    if (inc.unlisted > 0) line('b-unlisted', `${inc.unlisted} of those have no id here: the herdr popup has them.`);
  }
  if (s.pins.changed) line('b-changed', 'This changed since you opened it: a send will be refused. Go back and open it again.');
  if (s.change) line('b-look', s.change);
  for (const [i, n] of s.notices.entries()) line(`b-note-${i}`, n.text, n.severity === 'warning' ? t.blocked : t.muted);
  if (s.terminalHint !== null && mod.status.root) line('b-term', `In a terminal: sh ${mod.status.root}/scripts/launch cli app show --question ${s.terminalHint}`, t.muted);
  return lines;
}

// The closed rows that really are finished. "sending…" and a held answer get no tick.
const DONE = new Set(['answered', 'dismissed', 'acknowledged', 'hidden', 'delivered', 'cleared']);

function rowLine(t, mod, entry, n) {
  const p = mod.session.popup;
  const d = describe(mod, entry, Date.now() / 1000);
  const selected = p.view.cursor === entry;
  const closed = p.closed.get(entry);
  const color = closed !== undefined ? t.dim : d.warn ? t.blocked : t.foreground;
  const done = DONE.has(closed) ? '✓ ' : '';
  return v_flex().id(`row-${entry}`).gap(4).px(16).py(10).border_t_1().border_color(t.border).when(selected, el => el.bg(t.selected))
    .cursor_pointer().when(!selected, el => el.hover(s => s.bg(t.hover))).on_click((_e, cx) => mod.onRow(entry, cx))
    .child(h_flex().items_center().gap(8).text_size(13.5).text_color(color)
      .child(div().w(12).font_family(t.mono).text_size(12).text_color(t.accent).child(selected ? '❯' : ''))
      .child(div().font_family(t.mono).text_size(12).text_color(t.muted).child(d.mark ?? n))
      .child(div().flex_1().truncate().child(closed === undefined ? d.title : `${done}${d.title} · ${closed}`))
      .when(fresh(p, entry), el => el.child(div().text_size(11.5).text_color(t.accent).child('new')))
      .child(div().flex_none().font_family(t.mono).text_size(11.5).text_color(t.muted).child(d.meta)))
    .child(div().pl(20).truncate().text_size(12).text_color(t.muted).child(d.sub));
}

function listBody(mod, t) {
  const s = mod.session;
  const { entries } = s.popup;
  let n = 0;
  const rows = entries.map(e => {
    const numbered = !(e.startsWith('alert:') || e.startsWith('ralert:'));
    if (numbered) n += 1;
    return rowLine(t, mod, e, numbered ? `${n}.` : '');
  });
  const others = s.book.unknown().map(r => v_flex().id(`row-${r.key}`).px(16).py(10).border_t_1().border_color(t.border).text_size(13.5).text_color(t.muted).child(r.key));
  const count = entries.filter(e => e.startsWith('group:') || e.startsWith('remote:')).length;
  return [
    h_flex().flex_none().items_center().px(16).pt(14).pb(10).text_size(14).font_semibold().text_color(t.foreground)
      .child(div().flex_1().child('Open questions'))
      .child(div().font_family(t.mono).text_size(11.5).font_normal().text_color(t.muted).child(`${count} ${count === 1 ? 'group' : 'groups'}`)),
    v_flex().id('rows').flex_1().min_h(0).overflow_y_scrollbar().children(windowed(t, rows, entries.indexOf(s.popup.view.cursor), fit(mod, 150, 66))).children(others),
    footer(t, [['⏎', 'open'], ['g', 'go to agent'], ['d', 'dismiss'], ['r', 'open ⚠ row'], ...(s.incomplete?.ids.length ? [['o', 'left out']] : [])]),
  ];
}

function emptyBody(mod, t) {
  const whole = mod.session.empty;
  const arrived = mod.session.popup.entries.length > 0;
  const title = arrived ? 'New questions arrived. Tab shows them.' : whole ? 'No open questions' : 'Nothing listed here, but the inbox is not empty';
  return [v_flex().flex_1().items_center().justify_center().gap(6).child(div().text_size(15).text_color(whole || arrived ? t.muted : t.blocked).child(title))
    .when(!whole && mod.session.book.unknown().length > 0, el => el.children(mod.session.book.unknown().map(r => div().text_color(t.muted).child(r.key))))];
}

function plain(t, title, detail) {
  return [v_flex().flex_1().p(18).gap(12).text_size(13)
    .child(div().text_size(15).font_semibold().text_color(t.foreground).child(title))
    .child(div().text_color(t.muted).child(detail))];
}

function waiting(mod, t, words) {
  const s = mod.session;
  if (s.problem === null) return plain(t, words.title, words.detail);
  return plain(t, 'Inbox unavailable', s.errors[0] ?? `There is no inbox feed to read (${s.problem}).`);
}

function confirmBar(mod, t) {
  const c = mod.session.popup.confirm;
  if (!c) return [];
  return [h_flex().flex_none().items_center().gap(14).px(22).py(12).border_t_1().border_color(t.border).text_size(13).text_color(t.blocked)
    .child(div().flex_1().child(`Dismiss ${c.what} without answering?`))
    .children(keys(t, [['y', 'dismiss'], ['any other key', 'keep']]))];
}

function body(mod, t) {
  const s = mod.session;
  if (s.picker) return pickerBody(mod, t);
  const v = s.popup.view;
  if (v.kind === 'list') return listBody(mod, t);
  if (v.kind === 'none') return emptyBody(mod, t);
  if (v.kind === 'failed') return failedBody(mod, t);
  if (v.kind === 'alert') return alertBody(mod, t);
  if (v.kind === 'gone' && s.popup.mode !== 'group') return goneBody(mod, t);
  return groupBody(mod, t);
}

export const CONTEXT = 'HerdrInbox';
export const ESCAPE = 'herdr_inbox_escape';

export function render(mod, t) {
  const s = mod.session;
  const root = v_flex().id('herdr-inbox').key_context(CONTEXT).on_action(ESCAPE, (_a, cx) => mod.onEscape(cx)).track_focus(mod.handle).on_key_down((e, cx) => mod.onKey(e, cx)).on_key_up(e => mod.onKeyUp(e)).on_size_change((size, cx) => { mod.size = size; cx.notify(); }).size_full().min_h(0).bg(t.popover).text_size(13);
  if (mod.status.state !== 'ready') {
    const words = mod.wordsNow();
    return root.children(plain(t, words.title, words.detail));
  }
  if (!s.popup) return root.children(waiting(mod, t, mod.wordsNow()));
  const lines = banners(mod, t);
  const confirm = confirmBar(mod, t);
  mod.below = lines.length * 30 + confirm.length * 46; // what the rows must leave room for
  return root.children(body(mod, t)).children(confirm).children(lines);
}
