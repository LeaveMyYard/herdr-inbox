// The rows the overlay knows now, and the model's snapshot of them. The feed holds open rows only, so
// this also remembers what left: a question gone from the feed is shown as closed, under the word a
// verb gave for it when one did. A row left out to fit the size limit is not closed, and stays.

import { emptyRemote } from './model/data.js';
import { absentMeansClosed } from './feed.js';

const LOCAL = new Set(['group', 'failed', 'alert']);
const REMOTE = new Set(['remote_group', 'remote_outbox', 'remote_problem', 'remote_alert']);

// What `g` needs, as one opaque string: the model only passes it back. Any other host reads as unknown.
export function paneOf(asker, questionId) {
  if (asker.host === 'herdr-core' && asker.terminal) return `core:${asker.terminal}`;
  if (asker.host === 'herdr-tui') return questionId === null ? 'tui-alert:' : `tui:${questionId}`;
  return null;
}

function modelQuestion(q, row) {
  return {
    ...q, group: row.group, status: 'open', pane_id: paneOf(row.asker, q.id), tab_label: row.asker.tab_label,
    group_title: row.title, created_at: row.created_at, delivery_error: null, delivered: false, answer: null,
  };
}

function failedError(d) {
  if (d.state === 'agent_gone') return `agent gone: ${d.reason ?? ''}`;
  return d.reason ?? d.state;
}

function failedQuestion(row) {
  return {
    id: row.question_id, group: row.group, status: 'answered', pane_id: paneOf(row.asker, row.question_id), options: [], multi_select: false,
    draft: null, delivery_error: failedError(row.delivery), delivered: false, tab_label: row.asker.tab_label, asked_by: row.asked_by,
    header: row.header, question: row.question, context: null, group_title: null, answer: row.answer, created_at: row.answered_at,
  };
}

function modelAlert(row) {
  return {
    id: row.id, kind: row.alert_kind, status: 'open', pane_id: paneOf(row.asker, null), severity: row.severity, title: row.title,
    message: row.message, tab_label: row.asker.tab_label, asked_by: row.asked_by, created_at: row.created_at,
  };
}

function remoteQuestion(row, q) {
  const key = row.key.slice('remote:'.length + row.machine.id.length + 1);
  const bar = key.lastIndexOf('|');
  const question = { ...q, group: Number(key.slice(bar + 1)), status: 'open', pane_id: null, delivery_error: null, delivered: false, group_title: row.title };
  return { machine_id: row.machine.id, db_id: key.slice(0, bar), qkey: String(q.id), question };
}

function addRemote(remote, row) {
  const machine = row.machine;
  if (!remote.machines.some(m => m.machine_id === machine.id)) {
    remote.machines.push({ machine_id: machine.id, label: machine.label, offline: machine.offline });
  }
  const machine_id = machine.id;
  if (row.kind === 'remote_group') {
    const questions = row.questions.map(q => remoteQuestion(row, q));
    remote.groups.push({ key: row.key.slice('remote:'.length), machine_id, questions });
    remote.mirror.push(...questions.map(q => ({ machine_id, remote_id: q.question.id, db_id: q.db_id, qkey: q.qkey })));
  } else if (row.kind === 'remote_alert') {
    const alert = { id: row.id, kind: row.alert_kind, status: 'open', pane_id: null, severity: row.severity, title: row.title, message: row.message };
    remote.alerts.push({ machine_id, db_id: '', akey: String(row.id), alert });
  } else if (row.kind === 'remote_outbox') {
    remote.outbox.push({ ...row, request_id: row.key.slice('routbox:'.length), machine_id, clearable: false, expires: row.expires_at });
  } else {
    remote.problems.push({ ...row, machine_id, label: machine.label ?? machine.id });
  }
}

const questionIds = row => (row.kind === 'group' ? row.questions.map(q => q.id) : row.kind === 'failed' ? [row.question_id] : []);

export class Book {
  rows = new Map(); // key → the row, in the feed's order
  words = new Map(); // question id, or `alert:<id>`: the status a verb said it closed with
  lost = new Map(); // question id → its last model question
  lostAlerts = new Map();
  last = new Map(); // key → the row as it last was, for the words on a closed row

  row(key) {
    return this.rows.get(key);
  }

  // The row holding this question, open or undelivered.
  rowOf(qid) {
    return [...this.rows.values()].find(r => questionIds(r).includes(qid));
  }

  // The local rows, each by its own key; an unknown kind has no place in the model.
  unknown() {
    return [...this.rows.values()].filter(r => !LOCAL.has(r.kind) && !REMOTE.has(r.kind));
  }

  // A new feed replaces the rows, but a row it left out stays as it was known.
  apply(feed) {
    const next = new Map(feed.entries.map(row => [row.key, row]));
    for (const [key, row] of this.rows) {
      if (next.has(key)) {
        this.dropped(row, next.get(key));
        continue;
      }
      if (absentMeansClosed(row, feed)) this.forget(row);
      else {
        const kept = this.stillOpen(row, feed);
        this.dropped(row, kept);
        next.set(key, { ...kept, left_out: true });
      }
    }
    this.rows = next;
  }

  // A group the feed still holds may have lost questions: they closed, and were not removed.
  dropped(row, now) {
    if (row.kind !== 'group' || now.kind !== 'group') return;
    for (const q of row.questions.filter(x => !now.questions.some(y => y.id === x.id))) this.lost.set(q.id, modelQuestion(q, row));
  }

  // A kept group holds only the questions the feed still names, when it names them all.
  stillOpen(row, feed) {
    const { questions, question_ids: ids } = feed.omitted;
    if (row.kind !== 'group' || ids.length < questions) return row;
    return { ...row, questions: row.questions.filter(q => ids.includes(q.id)) };
  }

  // The row, or what it last was.
  shown(key) {
    return this.rows.get(key) ?? this.last.get(key);
  }

  forget(row) {
    if (!this.last.has(row.key) || row.questions?.length > 0) this.last.set(row.key, row);
    if (row.kind === 'group') for (const q of row.questions) this.lost.set(q.id, modelQuestion(q, row));
    else if (row.kind === 'failed') this.lost.set(row.question_id, failedQuestion(row));
    else if (row.kind === 'alert') this.lostAlerts.set(row.id, modelAlert(row));
  }

  // A row read with `app show`: it stays until a feed names it or says it closed.
  adopt(row) {
    this.rows.set(row.key, { ...row, left_out: !this.rows.get(row.key) || this.rows.get(row.key).left_out === true });
  }

  // What a verb's success did to the row.
  applied(key, { token, done, verb, detail, cleared }) {
    const row = this.rows.get(key);
    if (!row) return;
    this.last.set(key, { ...row });
    if (verb === 'draft') {
      const q = row.questions.find(x => x.id === done[0]);
      if (q) q.draft = cleared ? null : detail;
    } else if (row.kind === 'group') {
      for (const q of row.questions.filter(x => done.includes(x.id))) {
        this.words.set(q.id, verb === 'dismiss' ? 'dismissed' : 'answered');
        this.lost.set(q.id, modelQuestion(q, row));
      }
      row.questions = row.questions.filter(q => !done.includes(q.id));
    }
    if (verb === 'retry') row.retries += 1;
    if (token === null) {
      this.forget(row);
      this.rows.delete(key);
    } else {
      row.token = token;
    }
  }

  // A verb said one question of a row closed; the others are still open.
  questionClosed(key, qid, word) {
    const row = this.rows.get(key);
    const q = row?.questions.find(x => x.id === qid);
    if (!q) return;
    this.words.set(qid, word);
    this.lost.set(qid, modelQuestion(q, row));
    row.questions = row.questions.filter(x => x.id !== qid);
  }

  // A verb said this row closed: `closed` is its word for why.
  closed(key, word) {
    const row = this.rows.get(key);
    if (!row) return;
    for (const id of questionIds(row)) this.words.set(id, word);
    if (row.kind === 'alert') this.words.set(key, word);
    this.forget(row);
    this.rows.delete(key);
  }

  snapshot() {
    const rows = [...this.rows.values()];
    const groups = rows.filter(r => r.kind === 'group');
    const failed = rows.filter(r => r.kind === 'failed');
    const alerts = rows.filter(r => r.kind === 'alert');
    const questions = [...groups.flatMap(r => r.questions.map(q => modelQuestion(q, r))), ...failed.map(failedQuestion)];
    const live = new Set(questions.map(q => q.id));
    for (const [id, q] of this.lost) {
      if (live.has(id)) continue;
      const word = this.words.get(id);
      const status = word === 'delivered' || word === undefined ? 'no longer open' : word;
      questions.push({ ...q, status, delivered: word === 'delivered' });
    }
    const liveAlerts = new Set(alerts.map(a => a.id));
    const gone = [...this.lostAlerts.values()].filter(a => !liveAlerts.has(a.id));
    const remote = emptyRemote();
    for (const row of rows.filter(r => REMOTE.has(r.kind))) addRemote(remote, row);
    return {
      questions,
      open: groups.flatMap(r => r.questions.map(q => q.id)),
      failed: failed.map(r => r.question_id),
      alerts: [...alerts.map(modelAlert), ...gone.map(a => ({ ...a, status: this.words.get(`alert:${a.id}`) ?? 'closed' }))],
      open_alerts: alerts.map(r => r.id),
      remote,
    };
  }
}
