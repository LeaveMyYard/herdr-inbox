// Inside a group: the header, the tabs, a question with its options and preview, the Submit tab.

import { div } from 'gpui-kit';
import { h_flex, v_flex } from 'gpui-base';
import { hasPreview, question, tabId } from './model/data.js';
import { allTabs, openQids } from './model/groups.js';
import { boxValue } from './model/textbox.js';
import { viewQuestion } from './model/remote.js';
import { targets } from './model/review.js';
import { button, clickable, fit, footer, windowed } from './view-parts.js';
import { askerLabel } from './view-text.js';
import { age } from './words.js';

const RECOMMENDED = / \(recommended\)$/i;

function header(mod, t, qid) {
  const { popup, book } = mod.session;
  const row = book.row(popup.entry);
  const asker = row?.asker ? askerLabel(mod, row.asker) : row?.machine?.label ?? row?.machine?.id;
  const q = qid === null ? null : viewQuestion(popup, qid);
  const when = row?.created_at ? `${age(row.created_at, Date.now() / 1000)} ago`.replace('now ago', 'now') : null;
  const parts = [asker, q?.asked_by && `asked by ${q.asked_by}`, when].filter(Boolean);
  return v_flex().flex_none().gap(6).px(22).pt(18).pb(12)
    .child(div().text_size(18).font_semibold().text_color(t.foreground).truncate().child(row?.title ?? q?.header ?? `Question Q${qid}`))
    .child(h_flex().gap(10).font_family(t.mono).text_size(11.5).text_color(t.muted)
      .when(qid !== null, el => el.child(div().text_color(t.accent).child(`Q${qid}`)))
      .children(parts.map(p => div().child(p))));
}

// The question tabs on the left; the Submit tab, the last step, set apart on the right with its count.
function tabs(mod, t) {
  const { popup: p } = mod.session;
  const all = allTabs(p);
  if (p.mode !== 'group' || all.length < 2) return [];
  const click = tab => (_e, cx) => mod.onTab(tab, cx);
  const chip = tab => {
    const on = tab === p.current;
    const q = question(p.data, tabId(tab));
    const closed = p.gone.has(tab);
    const text = q?.header ?? `Q${tabId(tab)}`;
    const color = on ? t.foreground : closed ? t.dim : q?.draft ? t.ansi2 : t.muted;
    return div().id(`tab-${tab}`).px(12).py(9).text_size(13).text_color(color).border_b_2().border_color(on ? t.accent : t.popover)
      .cursor_pointer().hover(el => el.text_color(t.foreground)).on_click(click(tab)).child(closed ? `${text} (closed)` : text);
  };
  const open = openQids(p);
  const answered = open.filter(id => question(p.data, id)?.draft).length;
  const submit = () => {
    const on = p.current === 'submit';
    return h_flex().id('tab-submit').flex_none().items_center().gap(8).my(5).px(12).py(4).rounded(7).border_2().cursor_pointer()
      .border_color(on ? t.accent : answered > 0 ? t.solid : t.input).bg(on || answered > 0 ? t.solid : t.raised)
      .text_color(on || answered > 0 ? t.solid_foreground : t.muted).hover(el => el.border_color(t.accent)).on_click(click('submit'))
      .child(div().text_size(13).font_semibold().child('Submit ✓'))
      .child(div().font_family(t.mono).text_size(11.5).child(`${answered}/${open.length} answered`));
  };
  return [h_flex().flex_none().items_center().gap(4).px(22).border_b_1().border_color(t.border)
    .children(all.filter(tab => tab !== 'submit').map(chip)).child(div().flex_1()).children(all.includes('submit') ? [submit()] : [])];
}

function optionRow(mod, t, v, q, i) {
  const o = q.options[i];
  const on = v.cursor === i;
  const picked = v.picked.includes(i);
  const rec = RECOMMENDED.test(o.label);
  const note = v.notes.get(i);
  return h_flex().id(`opt-${i}`).gap(8).px(12).py(9).rounded(9).border_1().border_color(on ? t.accent : t.border).bg(on ? t.selected : t.raised)
    .map(clickable(t, on, (_e, cx) => mod.onRow(i, cx)))
    .child(div().w(22).flex_none().font_family(t.mono).text_size(12).text_color(on ? t.accent : t.muted).child(v.multi_select ? (picked ? '[x]' : '[ ]') : String(i + 1)))
    .child(v_flex().flex_1().min_w(0).gap(2)
      .child(h_flex().gap(8).items_center().text_size(13.5).text_color(t.foreground).child(o.label.replace(RECOMMENDED, ''))
        .when(rec, el => el.child(div().px(5).py(3).rounded(4).font_family(t.mono).text_size(10).font_semibold().text_color(t.ansi2).bg(t.surface).child('recommended'))))
      .when(Boolean(o.description), el => el.child(div().text_size(12.5).text_color(t.muted).child(o.description)))
      .when(Boolean(note), el => el.child(div().text_size(12.5).text_color(t.accent).child(`Note: ${note}`))));
}

function plainRow(t, id, n, label, detail, on, click = null) {
  return h_flex().id(id).when(click !== null, clickable(t, on, click)).gap(8).px(12).py(9).rounded(9).border_1().border_color(on ? t.accent : t.border).bg(on ? t.selected : t.raised)
    .child(div().w(22).flex_none().font_family(t.mono).text_size(12).text_color(on ? t.accent : t.muted).child(n))
    .child(v_flex().flex_1().min_w(0).gap(2).child(div().text_size(13.5).text_color(t.foreground).child(label))
      .when(Boolean(detail), el => el.child(div().text_size(12.5).text_color(t.muted).child(detail))));
}

function typed(box) {
  const value = boxValue(box);
  const at = Array.from(value).slice(0, box.cursor).join('');
  return box.selected ? `[${value}]` : `${at}▏${Array.from(value).slice(box.cursor).join('')}`;
}

function noteField(t, v) {
  const e = v.editing;
  const text = e ? typed(e.box) : e === null && v.text ? v.text : 'Add a note for the agent…';
  const label = e?.kind === 'text' ? 'Your answer' : e?.kind === 'note' ? `Note on option ${e.i + 1}` : null;
  return h_flex().flex_none().items_center().gap(8).min_h(34).px(12).rounded(8).border_1().border_color(e ? t.accent : t.input).text_size(13)
    .text_color(e || v.text ? t.foreground : t.dim).when(label !== null, el => el.child(div().flex_none().text_color(t.muted).child(label))).child(div().flex_1().child(text));
}

function questionBody(mod, t, v) {
  const { popup: p, full } = mod.session;
  const shown = viewQuestion(p, v.qid);
  const whole = full.get(v.qid);
  const q = whole ? { ...shown, ...whole } : shown;
  const n = q.options.length;
  const rows = [plainRow(t, 'other', String(n + 1), 'Something else…', v.text ?? 'Type your own answer.', v.cursor === n, (_e, cx) => mod.onRow(n, cx))];
  const last = v.rows[n + 1];
  rows.push(plainRow(t, last, last === 'chat' ? String(n + 2) : '', last === 'chat' ? 'Chat about this' : v.last ? 'Submit' : 'Next', last === 'chat' ? 'Go to the agent' : null, v.cursor === n + 1, (_e, cx) => mod.onRow(n + 1, cx)));
  const preview = q.options[v.cursor]?.preview ?? q.options.find(o => o.preview)?.preview;
  const all = [...q.options.map((_, i) => optionRow(mod, t, v, q, i)), ...rows];
  const text = v_flex().id('q').flex_1().min_h(96).overflow_y_scrollbar().gap(12)
    .child(h_flex().gap(8).items_center()
      .when(Boolean(q.header), el => el.child(div().px(6).py(4).rounded(4).font_family(t.mono).text_size(10.5).font_semibold().text_color(t.ansi4).bg(t.surface).child(q.header.toUpperCase())))
      .child(div().font_family(t.mono).text_size(11.5).text_color(t.muted).child(v.multi_select ? 'pick any' : 'pick one')))
    .child(div().text_size(15).text_color(t.foreground).child(q.question))
    .when(Boolean(q.context), el => el.child(div().text_size(12.5).text_color(t.muted).child(q.context)))
    .when(q.cut === true && p.mode === 'group', el => el.child(div().text_size(12).text_color(t.blocked).child('This text was cut to fit the feed. Press f to read the whole question.')));
  const choices = v_flex().flex_none().gap(6).children(windowed(t, all, v.cursor, fit(mod, 400, q.options.some(o => o.description) ? 70 : 48)));
  const column = v_flex().flex_1().min_w(0).min_h(0).gap(12).px(22).py(18).child(text).child(choices).child(noteField(t, v));
  const side = hasPreview(q) ? [v_flex().id('preview').w(300).flex_none().min_h(0).gap(10).p(18).bg(t.surface).border_l_1().border_color(t.border).overflow_y_scrollbar()
    .child(div().text_size(11).font_medium().text_color(t.dim).child(`PREVIEW · OPTION ${Math.min(v.cursor + 1, n)}`))
    .child(div().rounded(8).p(12).bg(t.background).border_1().border_color(t.border).font_family(t.mono).text_size(12).text_color(t.foreground).child(preview ?? ''))
    .child(div().text_size(12.5).text_color(t.muted).child('Shown when the agent attaches a preview to an option.'))] : [];
  return h_flex().flex_1().min_h(0).items_stretch().child(column).children(side);
}

// One of the Submit tab's two actions: Submit answers is the primary one, solid once anything is answered.
function action(mod, t, v, target, n, label, detail, primary) {
  const on = v.cursor === target;
  return h_flex().id(target).items_center().gap(10).px(14).py(10).rounded(9).border_2().cursor_pointer()
    .border_color(on ? t.accent : primary ? t.solid : t.input).bg(primary ? t.solid : t.raised).text_color(primary ? t.solid_foreground : t.foreground)
    .hover(el => el.border_color(t.accent)).on_click((_e, cx) => mod.onRow(target, cx))
    .child(div().w(14).flex_none().font_family(t.mono).text_size(12).child(n))
    .child(v_flex().flex_1().min_w(0).gap(2).child(div().text_size(14).font_semibold().child(label))
      .when(Boolean(detail), el => el.child(div().text_size(12.5).opacity(0.8).child(detail))));
}

function reviewBody(mod, t, v) {
  const { popup: p } = mod.session;
  const items = v.items.map(id => {
    const q = question(p.data, id);
    const d = q?.draft;
    const answer = d ? [...d.choices, ...(d.text ? [d.text] : [])].join(', ') : 'No answer yet: it stays open';
    const target = `item:${id}`;
    return plainRow(t, `item-${id}`, '', `Q${id} · ${q?.header ?? q?.question.split('\n')[0] ?? ''}`, answer, v.cursor === target, (_e, cx) => mod.onRow(target, cx));
  });
  const ready = v.drafted.length > 0;
  const sends = ready ? `Sends ${v.drafted.length} of ${v.items.length}` : 'Nothing answered yet';
  const actions = h_flex().id('review-actions').flex_none().gap(8).pt(12).border_t_1().border_color(t.border)
    .child(action(mod, t, v, 'submit', '1', 'Submit answers ✓', sends, ready).flex_1())
    .child(action(mod, t, v, 'cancel', '2', 'Cancel', 'Sends nothing', false).flex_none().w(200));
  const at = v.cursor?.startsWith('item:') ? targets(v).indexOf(v.cursor) : 0;
  return v_flex().id('review').flex_1().min_h(0).gap(6).px(22).py(18)
    .child(div().pb(6).text_size(15).text_color(t.foreground).child('Review your answers'))
    .child(v_flex().flex_1().min_h(0).gap(6).children(windowed(t, items, at, fit(mod, 340, 70))))
    .child(actions);
}

export function groupBody(mod, t) {
  const { popup: p } = mod.session;
  const v = p.view;
  const qid = v.kind === 'question' ? v.qid : p.current?.startsWith('q:') ? tabId(p.current) : null;
  const main = v.kind === 'question' ? questionBody(mod, t, v) : v.kind === 'review' ? reviewBody(mod, t, v)
    : v_flex().flex_1().items_center().justify_center().gap(6)
      .child(div().text_size(15).text_color(t.foreground).child(`${v.ref} was closed (${v.reason})`))
      .child(div().text_color(t.muted).child('Esc goes back, Tab moves on.'));
  const hint = p.mode === 'remote' ? [['esc', 'back (read-only here)']] : [
    ...(v.kind === 'question' ? [[`1-${Math.min(v.rows.filter(r => r !== 'submit').length, 9)}`, 'pick'], ['n', 'note']] : []),
    ...(allTabs(p).length > 1 ? [['tab', 'next question']] : []), ['g', 'go to agent'], ['d', 'dismiss'], ['esc', 'back'],
  ];
  const action = v.kind === 'review' && v.cursor === 'submit' ? 'Submit' : p.single ? 'Send' : 'Next';
  const later = button(t, 'later', 'Later', false, (_e, cx) => mod.onButton('escape', cx));
  const enter = button(t, 'enter', `${action} ⏎`, true, (_e, cx) => mod.onButton('enter', cx));
  return [header(mod, t, qid), ...tabs(mod, t), main, footer(t, hint, p.mode === 'remote' ? [later] : [later, enter])];
}
