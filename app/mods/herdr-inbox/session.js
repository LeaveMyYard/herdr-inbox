// The overlay's rules around the feed and the keys, with no host in it. `Steps` runs what they lead to.
// A key waits for nothing: while a verb runs it is dropped, so nothing queues behind one.

import { incompleteness, isStale, parseFeed } from './feed.js';
import { latinKey, repeatCounts } from './input.js';
import { Popup } from './model/popup.js';
import { Steps } from './steps.js';
import { changeWords } from './words.js';

const seconds = ms => ms / 1000;
const ESCAPE_REPEAT_MS = 150; // the host's action has no is_held: a second Esc this soon is the key held

export class Session extends Steps {
  constructor({ question = null, hold = true } = {}) {
    super();
    this.link = question === null ? null : { qid: question, hold };
    this.feed = null;
    this.stale = false;
    this.problem = null;
    this.errors = [];
    this.lastEscape = -Infinity;
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
    if (this.popup) this.run(this.popup.handle({ poll: this.book.snapshot() }, now), now);
    else if (!this.showing) this.start(now);
    if (this.repin) {
      this.repin = false;
      this.pins.look(this.popup);
    }
  }

  get incomplete() {
    return this.feed ? incompleteness(this.feed) : null;
  }

  // `entries: []` means an empty inbox only on a whole, good feed.
  get empty() {
    return this.feed !== null && this.problem === null && this.book.rows.size === 0 && this.incomplete === null;
  }

  // What changed, for the screen that makes the operator look again.
  get change() {
    if (!this.lookAgain) return null;
    const { entry, before } = this.lookAgain;
    return changeWords(before === null ? null : JSON.parse(before), this.book.row(entry) ?? null);
  }

  // -- opening

  // A deep link opens the row holding its question. Absence is no closure: on a feed that left
  // questions out, `show` is asked first, and the popup waits for its answer.
  start(now) {
    const { link, feed } = this;
    if (link && !link.tried && !this.book.rowOf(link.qid) && feed.omitted.questions > 0) {
      link.tried = true;
      this.showing = { qid: link.qid, then: 'open' };
      return;
    }
    this.popup = new Popup({ snapshot: this.book.snapshot(), focus_pane: '', deeplink: link ? this.deepLink() : null }, now);
    this.run(this.popup.first, now);
  }

  deepLink() {
    const { qid, hold, unread } = this.link;
    const row = this.book.rowOf(qid);
    if (row) return { target: row.key, qid, hold, missing: '' };
    return { target: null, qid: null, hold, missing: unread ? `Q${qid} could not be read` : `Q${qid} is no longer open` };
  }

  // -- keys

  // Nothing acts while a verb or a read runs, or once the overlay is closing.
  get idle() {
    return !this.batch && !this.showing && !this.closing && this.popup !== null;
  }

  graced(now) {
    const { grace_until: grace } = this.popup.timers;
    return grace !== null && now < grace;
  }

  // The key was taken, so what was said about the last one has been seen.
  seen(now) {
    if (this.graced(now)) return;
    this.lookAgain = null;
    this.notices = [];
    this.terminalHint = null;
  }

  // Esc, from the host's action: it has no event to say the key is held.
  escape(now) {
    const repeat = now - this.lastEscape < ESCAPE_REPEAT_MS;
    this.lastEscape = now;
    if (!repeat) this.key('escape', now);
  }

  // `typed` is the key as it writes, `acting` as it commands: they differ on a non-Latin layout.
  key(typed, now, { held = false, acting = typed } = {}) {
    if (!this.idle) return;
    const name = this.popup.view.box_focused ? typed : latinKey(acting);
    if (held && !repeatCounts(this.popup, this.picker !== null, name)) return;
    if (name === 'escape' && this.picker) {
      this.picker = null;
      return;
    }
    if (this.picker) return this.pickerKey(name);
    this.keepReshow();
    if (this.reshow !== null && name !== 'escape') {
      this.showing = { qid: this.reshow.qid, then: 'reshown' };
      this.reshow = null;
      return;
    }
    if (this.sessionKey(name, now)) return;
    const { mode } = this.popup;
    const stays = this.incomplete || this.book.unknown().length > 0;
    if (mode === 'empty' && stays && !['escape', 'tab', 'shift+tab', 'left', 'right'].includes(name)) return;
    this.act(now, () => this.popup.handle({ key: name }, now));
  }

  act(now, event) {
    const before = this.popup.saved({ data: false });
    this.pins.keyStarts(this.popup);
    this.seen(now);
    this.run(event(), now, before);
    this.keepReshow();
  }

  // A left-out row's `show` is owed only while that row is on screen.
  keepReshow() {
    if (this.reshow && this.popup.entry !== this.reshow.entry) this.reshow = null;
  }

  // A click on a row: one action on that row, or nothing.
  click(row, now) {
    if (this.idle && !this.picker && this.reshow === null) this.act(now, () => this.popup.pick(row, now));
  }

  // A footer button does what its key does, but never closes a confirmation or submits a text box.
  button(name, now) {
    if (this.idle && !this.picker && !this.popup.confirm && !this.popup.view.box_focused && !this.graced(now)) this.key(name, now);
  }

  // A paste goes into an open text box, and nowhere else: its letters are no keys to act on.
  paste(keys, now) {
    if (this.popup?.view.box_focused) for (const key of keys) this.key(key, now);
  }

  // Keys of the overlay that the model has no word for: the left-out list, a cut question in full, and
  // `r` on a ⚠ row, which opens it: its retry stays one deliberate key away.
  sessionKey(name, now) {
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
    if (name === 'r' && p.view.kind === 'list' && p.view.cursor?.startsWith('failed:')) {
      this.act(now, () => p.handle({ key: 'enter' }, now));
      return true;
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
}
