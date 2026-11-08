// Running what the model asked for. main.js asks `next()` for a command, carries it out and reports with
// `finish()`. A verb carries the token of what was shown, and a refused verb puts the screen back and
// sends nothing again: only the operator's next deliberate key can act.

import { Book } from './book.js';
import { readResult, showArgs, verbArgs } from './commands.js';
import { goodEntry } from './feed.js';
import { Pins } from './pins.js';
import { plan } from './plan.js';
import { closedWords, heldWords, knownDelivery, leftOutWords, noneOpenWords, queuedWords, refusalWords, skippedWords, unknownWords } from './words.js';

const READ_ONLY = 'Remote rows are read-only here: nothing was sent. Act on them in the herdr popup.';
const NOT_NOW = 'This row does not take that action now. Nothing was sent.';
const KEEP_TYPED_ON_STALE = true; // false shows the row afresh instead

const refusal = (code, message) => ({ kind: 'refused', code, message, closed: null });

export class Steps {
  constructor() {
    Object.assign(this, {
      book: new Book(), popup: null, batch: null, waiting: null, wrote: 0, notices: [], toasts: [], closing: false, reload: false,
      lookAgain: null, repin: false, reshow: null, picker: null, full: new Map(), showing: null, terminalHint: null,
    });
    this.pins = new Pins(this.book);
  }

  notice(text, severity = 'information') {
    this.notices = [...this.notices.slice(-2), { text, severity }];
  }

  // Said on screen and as a toast, which outlives the overlay.
  say(text, severity) {
    this.notice(text, severity);
    this.toasts.push({ text, severity });
  }

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
    if (nothing) this.settled({ done: [], wanted: 1 }, now);
    if (close) this.close();
  }

  close() {
    this.closing = true;
    const told = new Set(this.toasts.map(n => n.text));
    this.toasts.push(...this.notices.filter(n => n.severity === 'warning' && !told.has(n.text)));
  }

  // What main.js should do now: { args } for a verb or a read, { go } for go-to-agent, or null.
  next(now) {
    if (this.showing) return { show: this.showing, args: showArgs(this.showing.qid) };
    if (!this.batch) return null;
    const command = this.batch.commands[this.batch.at];
    if (command.go) return { go: command.go };
    const row = this.book.row(command.entry);
    const token = this.pins.token(command.entry);
    if (!row || !row.actions.includes(command.verb) || typeof token !== 'string') return this.refuse(command, refusal('read_only', NOT_NOW), now);
    if (row.kind === 'failed' && !knownDelivery(row)) return this.refuse(command, refusal('read_only', NOT_NOW), now);
    const args = verbArgs(command, token);
    if (args === null) return this.refuse(command, refusal('invalid', 'That answer is too long to send. Nothing was sent.'), now);
    return { verb: command, args };
  }

  // The result of what `next` gave.
  finish(step, result, now) {
    if (step.show) return this.finishShow(step.show, readResult(result, 'show', null), now);
    if (step.go) return this.finishGo(result, now);
    const read = readResult(result, step.verb.verb, step.verb.entry);
    if (read.kind !== 'done') return this.refuse(step.verb, read, now);
    const { verb, entry, detail, clear, wanted, sent } = step.verb;
    this.wrote = now / 1000;
    this.book.applied(entry, { token: read.token, done: read.done, verb, detail, cleared: clear });
    this.pins.moved(entry, read.token);
    if (sent) this.settled({ ...read, verb, wanted, entry }, now);
    this.advance(now);
  }

  // Something in a step threw: its effect is unknown, as for any result that can't be read.
  failed(step, why, now) {
    this.showing = null;
    if (this.batch) this.refuse(step.verb ?? {}, { kind: 'unknown', why }, now);
    else this.notice(unknownWords(why), 'warning');
  }

  // `result`: { error } when it could not go, { notice } when it should not, else it went.
  finishGo({ error = null, notice = null }, now) {
    if (notice) this.notice(notice);
    else {
      const fx = this.popup.handle({ focused: { pane: 'that agent', error } }, now);
      for (const e of fx) if (e.notify) this.notice(e.notify.text, e.notify.severity);
      if (fx.some(e => e.exit)) this.batch.close = true;
    }
    this.advance(now);
  }

  // On to the next command, or the batch is over.
  advance(now) {
    const b = this.batch;
    b.at += 1;
    if (b.at < b.commands.length) return;
    this.batch = null;
    this.pins.follow(this.popup);
    if (b.close) {
      if (b.commands.some(c => c.sent) && this.incomplete?.counts.length > 0) this.say(leftOutWords(this.incomplete.counts), 'warning');
      this.close();
    }
    if (this.waiting) {
      const { text } = this.waiting;
      this.waiting = null;
      this.read(text, now);
    }
  }

  // A verb's words, and the model told: a `held` result never reads as sending, on the row either.
  settled({ done, wanted, verb = 'answer', delivery = null, skipped = [], entry = null }, now) {
    if (done.length === 0 && skipped.length === 0) this.notice(noneOpenWords, 'warning');
    else if (delivery === 'held') this.say(heldWords(verb, done), 'warning');
    else if (done.length > 0) this.notice(queuedWords(done));
    if (skipped.length > 0) this.say(skippedWords(skipped), 'warning');
    if (done.length > 0 && done.length + skipped.length < wanted) this.notice(closedWords(wanted - done.length - skipped.length), 'warning');
    if (!this.popup.running) return;
    const fx = this.popup.handle({ sent: { done, wanted, snapshot: this.book.snapshot() } }, now);
    if (delivery === 'held') this.popup.relabel(entry, 'recorded, nothing sent');
    else if (done.length === 0) this.popup.relabel(entry, 'nothing sent');
    if (fx.some(e => e.exit)) this.close();
  }

  // A verb did not go through, or may not have. Everything after it is dropped and the screen goes back
  // to before the key. The feed is read again; the verb is not.
  refuse(command, read, now) {
    const b = this.batch;
    this.batch = null;
    this.waiting = null;
    const stale = read.kind === 'refused' && read.code === 'stale';
    if (stale) this.staleRow(command, read);
    this.say(read.kind === 'unknown' ? unknownWords(read.why) : refusalWords(command.verb, read), 'warning');
    if (b?.before) this.run(this.popup.refused(b.before, this.book.snapshot(), now), now);
    else if (b) this.run(this.popup.handle({ poll: this.book.snapshot() }, now), now);
    if (stale && !KEEP_TYPED_ON_STALE) this.run(this.popup.remount(now), now);
    this.reload = true;
    return null;
  }

  // `closed` names the row or one question of it. Only a verb on one question can say which.
  staleRow(command, { closed }) {
    const row = this.book.row(command.entry);
    const before = this.pins.shown(command.entry) ?? (row ? JSON.stringify(row) : null);
    const [only, ...more] = command.qids ?? [];
    if (closed && row?.kind === 'group' && row.questions.length > 1) {
      if (only !== undefined && more.length === 0) this.book.questionClosed(command.entry, only, closed);
    } else if (closed) this.book.closed(command.entry, closed);
    else if (row?.left_out) this.reshow = { qid: row.questions?.[0]?.id, entry: command.entry };
    this.lookAgain = { entry: command.entry, before };
    this.repin = true;
  }

  // `show` answers for the question asked: a row of another shape or for another question is no answer.
  finishShow(what, read, now) {
    this.showing = null;
    const asked = read.kind === 'shown' && read.row.kind === 'group' && goodEntry(read.row) && read.row.questions.some(q => q.id === what.qid);
    if (read.kind === 'shown' && !asked) return this.showFailed(what, 'its result named another question', now);
    if (read.kind === 'unknown') return this.showFailed(what, read.why, now);
    if (read.kind === 'refused') {
      this.notice(read.closed ? `Q${what.qid} is no longer open (${read.closed}).` : refusalWords('show', read), 'warning');
      if (!this.popup) this.start(now);
      return;
    }
    if (what.then === 'full') return void this.full.set(what.qid, read.row.questions.find(q => q.id === what.qid));
    this.book.adopt(read.row);
    for (const q of read.row.questions) this.full.set(q.id, q);
    if (!this.popup) return this.start(now);
    this.run(this.popup.handle({ poll: this.book.snapshot() }, now), now);
    this.run(this.popup.showRow(read.row.key, now), now);
    this.pins.look(this.popup);
  }

  // `show` writes nothing, so a failed one changed nothing either.
  showFailed(what, why, now) {
    this.notice(`Q${what.qid} could not be read (${why}). Nothing was changed.`, 'warning');
    this.terminalHint = what.qid;
    if (this.link) this.link.unread = true;
    if (!this.popup) this.start(now);
  }
}
