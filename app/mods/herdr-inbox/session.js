// The overlay's rules between the feed, the model and the verbs, with no host in it: main.js asks
// `next()` for a command, runs it and reports with `finish()`. A verb carries the token of what was
// shown, keys wait while a verb runs, and a refused verb puts the screen back and sends nothing again.

import { Book } from './book.js';
import { readResult, showArgs, verbArgs } from './commands.js';
import { incompleteness, isStale, parseFeed } from './feed.js';
import { Pins } from './pins.js';
import { plan } from './plan.js';
import { Popup } from './model/popup.js';
import { changeWords, closedWords, heldWords, noneOpenWords, queuedWords, refusalWords, skippedWords, unknownWords } from './words.js';

const seconds = ms => ms / 1000;
const READ_ONLY = 'Remote rows are read-only here: nothing was sent. Act on them in the herdr popup.';

export class Session {
  constructor({ question = null, hold = true } = {}) {
    Object.assign(this, {
      book: new Book(), link: question === null ? null : { qid: question, hold }, popup: null, feed: null, stale: false, problem: null,
      errors: [], batch: null, waiting: null, wrote: 0, notices: [], toasts: [], closing: false, reload: false, lookAgain: null, repin: false,
      reshow: null, picker: null, full: new Map(), showing: null, terminalHint: null,
    });
    this.pins = new Pins(this.book);
  }

  notice(text, severity = 'information') {
    this.notices = [...this.notices.slice(-2), { text, severity }];
  }

  // -- the feed

  // A read of the feed file, or null if it was not there. Held back while a verb runs, and until a
  // feed newer than the mod's last write arrives: an older one would bring back what was just done.
  read(text, now) {
    if (this.batch) {
      this.waiting = { text };
      return;
    }
    const { feed, problem, errors } = parseFeed(text);
    this.problem = problem ?? null;
    if (!feed) {
      this.errors = errors;
      this.stale = true;
      return;
    }
    this.errors = feed.errors;
    this.stale = isStale(feed.generated_at, seconds(now));
    if (this.feed && feed.generated_at <= this.wrote) return;
    this.feed = feed;
    this.book.apply(feed);
    if (!this.popup) this.start(now);
    else this.run(this.popup.handle({ poll: this.book.snapshot() }, now), now);
    if (this.repin) {
      this.repin = false;
      this.pins.look(this.popup);
    }
  }

  // Whether the view may show the empty state: `entries: []` means empty only on a whole feed.
  get incomplete() {
    return this.feed ? incompleteness(this.feed) : null;
  }

  get empty() {
    return this.feed !== null && this.problem === null && this.book.rows.size === 0 && this.incomplete === null;
  }

  // -- opening

  // A deep link opens the row holding its question; a left-out question is read with `show` first.
  deepLink() {
    const { qid, hold } = this.link;
    const row = this.book.rowOf(qid);
    if (row) return { target: row.key, qid, hold, missing: '' };
    return { target: null, qid: null, hold, missing: `Q${qid} is no longer open` };
  }

  start(now) {
    if (this.link && !this.link.tried && !this.book.rowOf(this.link.qid) && this.feed.omitted.question_ids.includes(this.link.qid)) {
      this.link.tried = true;
      this.showing = { qid: this.link.qid, then: 'open' };
      return;
    }
    this.popup = new Popup({ snapshot: this.book.snapshot(), focus_pane: '', deeplink: this.link ? this.deepLink() : null }, now);
    this.run(this.popup.first, now);
  }

  // -- keys

  key(name, now) {
    if (this.batch || this.showing || this.closing || !this.popup) return;
    if (name === 'escape' && this.picker) {
      this.picker = null;
      return;
    }
    if (this.picker) return this.pickerKey(name);
    if (this.reshow !== null && name !== 'escape') {
      this.showing = { qid: this.reshow, then: 'reshown' };
      this.reshow = null;
      return;
    }
    if (this.sessionKey(name)) return;
    if (this.popup.mode === 'empty' && (this.incomplete || this.book.unknown().length > 0) && name !== 'escape') return;
    const before = this.popup.saved({ data: false });
    this.pins.keyStarts(this.popup);
    const fx = this.popup.handle({ key: name }, now);
    const { grace_until: grace } = this.popup.timers;
    if (grace === null || now >= grace) this.lookAgain = null;
    this.run(fx, now, before);
  }

  // Keys of the overlay that the model has no word for: the left-out list, and a cut question in full.
  sessionKey(name) {
    const p = this.popup;
    if (name === 'o' && (p.mode === 'list' || p.mode === 'empty') && this.incomplete?.ids.length > 0) {
      this.picker = { cursor: this.incomplete.ids[0] };
      return true;
    }
    if (name === 'f' && p.mode === 'group' && p.view.kind === 'question' && !p.view.box_focused) {
      const q = this.book.rowOf(p.view.qid)?.questions?.find(x => x.id === p.view.qid);
      if (q?.cut && !this.full.has(q.id)) {
        this.showing = { qid: q.id, then: 'full' };
        return true;
      }
    }
    return false;
  }

  // The left-out list: its cursor is an id, so the list changing under it can't move it to another.
  pickerKey(name) {
    const { ids } = this.incomplete ?? { ids: [] };
    const at = ids.indexOf(this.picker.cursor);
    if (name === 'up' || name === 'k') this.picker.cursor = ids[Math.max(at - 1, 0)] ?? null;
    else if (name === 'down' || name === 'j') this.picker.cursor = ids[Math.min(at + 1, ids.length - 1)] ?? null;
    else if (name === 'enter' && at >= 0) {
      this.showing = { qid: this.picker.cursor, then: 'row' };
      this.picker = null;
    }
  }

  // -- effects

  // The model's effects become one batch of commands, run in order under what the screen was before.
  run(fx, now, before = null) {
    const { refusedRemote, notices, commands, close, nothing } = plan(fx, this.book);
    if (refusedRemote) {
      this.notice(READ_ONLY, 'warning');
      return this.run(this.popup.handle({ remote_refused: { notice: null, snapshot: null } }, now), now);
    }
    for (const n of notices) this.notice(n.text, n.severity);
    if (commands.length > 0) {
      this.batch = { commands, before, close, at: 0 }; // the row keeps its pin: the model has already left it
      return;
    }
    this.pins.follow(this.popup);
    if (nothing) this.settle({ done: [], wanted: 1 }, now);
    if (close) this.close();
  }

  close() {
    this.closing = true;
    this.toasts.push(...this.notices.filter(n => n.severity === 'warning'));
  }

  // -- running commands

  // What main.js should do now: { args } for a verb, { go } for go-to-agent, or null.
  next(now) {
    if (this.showing) return { show: this.showing, args: showArgs(this.showing.qid) };
    if (!this.batch) return null;
    const command = this.batch.commands[this.batch.at];
    if (command.go) return { go: command.go };
    const row = this.book.row(command.entry);
    if (command.entry === null || !row || !row.actions.includes(command.verb)) {
      return this.refuse(command, { kind: 'refused', code: 'read_only', message: 'This row does not take that action now. Nothing was sent.', closed: null }, now);
    }
    const args = verbArgs(command, this.pins.token(command.entry));
    if (args === null) return this.refuse(command, { kind: 'refused', code: 'invalid', message: 'That answer is too long to send. Nothing was sent.', closed: null }, now);
    return { verb: command, args };
  }

  // The exec result of what `next` gave.
  finish(step, result, now) {
    if (step.show) return this.finishShow(step.show, readResult(result, 'show', null), now);
    if (step.go) return this.finishGo(step.go, result, now);
    const read = readResult(result, step.verb.verb, step.verb.entry);
    if (read.kind !== 'done') return this.refuse(step.verb, read, now);
    const { verb, entry, detail, clear, wanted, sent } = step.verb;
    this.wrote = seconds(now);
    this.book.applied(entry, { token: read.token, done: read.done, verb, detail, cleared: clear });
    this.pins.moved(entry, read.token);
    if (sent) this.settle({ ...read, verb, wanted }, now);
    this.advance(now);
  }

  // `result`: { error } when it could not go, { notice } when it should not, else it went.
  finishGo({ pane }, { error = null, notice = null }, now) {
    if (notice) this.notice(notice);
    else {
      const fx = this.popup.handle({ focused: { pane: 'that agent', error } }, now);
      for (const e of fx) if (e.notify) this.notice(e.notify.text, e.notify.severity);
      if (fx.some(e => e.exit)) this.batch.close = true;
    }
    this.advance(now, pane);
  }

  // On to the next command, or the batch is over.
  advance(now) {
    const b = this.batch;
    b.at += 1;
    if (b.at < b.commands.length) return;
    this.batch = null;
    this.pins.follow(this.popup);
    if (b.close) this.close();
    if (this.waiting) {
      const { text } = this.waiting;
      this.waiting = null;
      this.read(text, now);
    }
  }

  // A verb's words, and the model told: a `held` result never reads as sending.
  settle({ done, wanted, verb = 'answer', delivery = null, skipped = [] }, now) {
    if (done.length === 0 && skipped.length === 0) this.notice(noneOpenWords, 'warning');
    else if (delivery === 'held') this.say(heldWords(verb, done), 'warning');
    else if (done.length > 0) this.notice(queuedWords(done));
    if (skipped.length > 0) this.notice(skippedWords(skipped), 'warning');
    if (done.length > 0 && done.length + skipped.length < wanted) this.notice(closedWords(wanted - done.length - skipped.length), 'warning');
    if (!this.popup.running) return;
    const fx = this.popup.handle({ sent: { done, wanted, snapshot: this.book.snapshot() } }, now);
    if (fx.some(e => e.exit)) this.close();
  }

  say(text, severity) {
    this.notice(text, severity);
    this.toasts.push({ text, severity });
  }

  // A verb did not go through, or may not have. Everything after it is dropped, the screen goes back to
  // before the key, and the verb is not run again: only the operator's next deliberate key can act.
  refuse(command, read, now) {
    const b = this.batch;
    this.batch = null;
    if (read.kind === 'refused' && read.code === 'stale') this.staleRow(command, read);
    this.say(read.kind === 'unknown' ? unknownWords(read.why) : refusalWords(command.verb, read), 'warning');
    if (b?.before) this.run(this.popup.refused(b.before, this.book.snapshot(), now), now);
    else if (b) this.run(this.popup.handle({ poll: this.book.snapshot() }, now), now);
    this.closing = false;
    this.reload = true;
    return null;
  }

  staleRow(command, { closed }) {
    const row = this.book.row(command.entry);
    const before = this.pins.shown(command.entry) ?? (row ? JSON.stringify(row) : null);
    if (closed) this.book.closed(command.entry, closed);
    else if (row?.left_out) this.reshow = row.questions?.[0]?.id ?? null;
    this.lookAgain = { entry: command.entry, before };
    this.repin = true;
  }

  // What changed, for the screen that makes the operator look again.
  get change() {
    if (!this.lookAgain) return null;
    const { entry, before } = this.lookAgain;
    return changeWords(before === null ? null : JSON.parse(before), this.book.row(entry) ?? null);
  }

  finishShow(what, read, now) {
    this.showing = null;
    if (read.kind === 'shown') {
      if (what.then === 'full') {
        const q = read.row.questions.find(x => x.id === what.qid);
        if (q) this.full.set(q.id, q);
        return;
      }
      this.book.adopt(read.row);
      for (const q of read.row.questions) this.full.set(q.id, q);
    } else if (read.kind === 'refused') {
      this.notice(read.closed ? `Q${what.qid} is no longer open (${read.closed}).` : refusalWords('show', read), 'warning');
    } else {
      this.notice(unknownWords(read.why), 'warning');
      this.terminalHint = what.qid;
    }
    if (!this.popup) return this.start(now);
    if (read.kind !== 'shown') return;
    this.run(this.popup.handle({ poll: this.book.snapshot() }, now), now);
    this.run(this.popup.showRow(read.row.key, now), now);
    this.pins.look(this.popup);
  }
}
