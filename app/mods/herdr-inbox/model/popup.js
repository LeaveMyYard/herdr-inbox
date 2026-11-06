// The inbox popup as a pure state machine: keys, polls and results in, effects out. It decides which
// answer goes to which question, so every rule here replays the transition golden
// (tests/fixtures/popup-transitions.json, with popup-deviations.json). The rule above all: no
// background event changes what the next key does. It does no IO and reads no clock.

import { message } from './actions.js';
import { GRACE, VIEW_ARM, copied, emptyRemote, isAlert, kindOf } from './data.js';
import { backToList, openEntry, show, showEmpty, showList } from './groups.js';
import { takeKey } from './keys.js';
import { focused, poll, sent } from './poll.js';
import { pickRow } from './question.js';
import { currentEntries } from './rows.js';

export class Popup {
  constructor(open, now) {
    Object.assign(this, {
      data: { remote: emptyRemote(), ...open.snapshot },
      focus_pane: open.focus_pane ?? null, mode: 'empty', entries: [], closed: new Map(), arrived_at: new Map(), list_cursor: null,
      direct: false, held: false, held_at: 0, entry: null, tabs: [], current: null, gone: new Map(), single: false, gone_batch: null,
      timers: { grace_until: null, view_at: null, last_key_at: null, prev_key_at: null },
      view: { kind: 'none' }, focus: 'view', confirm: null, remote_open: null, remote_alert: null, undo: null, running: true, now, fx: [], queue: [],
    });
    this.out = { notify: (text, severity) => this.notify(text, severity), later: msg => this.later(msg) };
    this.first = this.start(open.deeplink ?? null);
  }

  // Where it opens: a deep link's row, the only group, the list, or nothing.
  start(link) {
    this.entries = currentEntries(this, this.focus_pane);
    if (link && link.target === null) this.notify(link.missing, 'information');
    if (link && link.target !== null && this.entries.includes(link.target)) {
      this.held = link.hold;
      this.held_at = this.now;
      openEntry(this, link.target, link.qid);
    } else if (this.entries.length === 0) {
      showEmpty(this);
    } else {
      const questions = this.entries.filter(e => !isAlert(e));
      if (questions.length === 1 && kindOf(questions[0]) === 'group') {
        this.direct = true;
        openEntry(this, questions[0], null);
      } else {
        showList(this, questions[0] ?? null);
      }
    }
    this.drain();
    return this.take();
  }

  // Handles one event at `now`; the effects to carry out, in order.
  handle(event, now) {
    this.now = now;
    const refused = 'remote_refused' in event;
    if (!refused) this.undo = null;
    if (this.running || refused) {
      if ('key' in event) takeKey(this, event.key);
      else if ('poll' in event) poll(this, event.poll);
      else if ('sent' in event) sent(this, event.sent.done, event.sent.wanted, event.sent.snapshot);
      else if ('focused' in event) focused(this, event.focused.pane, event.focused.error);
      else if ('data' in event) this.data = event.data;
      else if (refused) this.remoteRefused(event.remote_refused);
      this.drain();
    }
    return this.take();
  }

  take() {
    const { fx } = this;
    this.fx = [];
    return fx;
  }

  drain() {
    while (this.queue.length > 0) {
      const msg = this.queue.shift();
      if (this.running) message(this, msg);
    }
  }

  notify(text, severity) {
    this.fx.push({ notify: { text, severity } });
  }

  later(msg) {
    this.queue.push(msg);
  }

  exit() {
    this.fx.push({ exit: {} });
    this.running = false;
  }

  quit() {
    this.fx.push({ quit: {} });
    this.running = false;
  }

  // The state as it stands: what a refused action goes back to.
  saved({ data = true } = {}) {
    const { out, undo, fx, queue, first, data: rows, ...state } = this;
    return copied(data ? { ...state, data: rows } : state);
  }

  restore(state) {
    Object.assign(this, copied(state), { undo: null, fx: [], queue: [] });
  }

  checkpoint() {
    this.undo = this.saved();
  }

  // A remote action the IO layer refused: back to before it. The timers stay: a grace window the
  // action opened goes on guarding the keys already typed at a screen that just changed back.
  remoteRefused({ notice, snapshot }) {
    if (this.undo) {
      const { now, timers } = this;
      this.restore(this.undo);
      Object.assign(this, { now, timers });
    }
    if (snapshot) this.data = snapshot;
    if (notice) this.notify(notice, 'warning');
    if (this.mode === 'alert' && this.entry !== null && kindOf(this.entry) === 'ralert' && this.remote_alert) backToList(this, true);
  }

  // A local verb was refused, or its result is unknown: back to before the key that sent it (`state`
  // holds no data), with the data as it is now. Nothing is sent again: the view is armed anew and the keys already on their
  // way are dropped, so only a deliberate key after a look at the new data can act.
  refused(state, snapshot, now) {
    const { timers } = this;
    this.restore(state);
    Object.assign(this, { now, timers, confirm: null, running: true });
    poll(this, snapshot);
    this.timers.grace_until = now + GRACE;
    if ('armed_at' in this.view) this.view.armed_at = now + VIEW_ARM;
    this.drain();
    return this.take();
  }

  // A click on a row of the question on screen. It acts on exactly that row, or not at all: not under
  // a confirmation, not with a box open, not while the view is unarmed or held, not in the grace.
  pick(row, now) {
    this.now = now;
    this.undo = null;
    const v = this.view;
    const grace = this.timers.grace_until;
    const plain = v.kind === 'question' && !v.editing && !this.confirm && !this.held && now >= v.armed_at;
    if (!this.running || !plain || (grace !== null && now < grace)) return [];
    this.timers.prev_key_at = this.timers.last_key_at;
    this.timers.last_key_at = now;
    pickRow(v, row, this.out);
    this.drain();
    return this.take();
  }

  // What a row placeholder says, once the verb's result is known.
  relabel(entry, reason) {
    if (this.closed.has(entry)) this.closed.set(entry, reason);
  }

  // Shows the tab on screen again from the data: what a `stale` does if typed text is not kept.
  remount(now) {
    this.now = now;
    if (this.mode === 'group') show(this, this.current);
    return this.take();
  }

  // The operator asked for this row: the list, on it. A row that only just arrived stays unarmed.
  showRow(entry, now) {
    this.now = now;
    if (this.entries.includes(entry)) showList(this, entry);
    return this.take();
  }
}
