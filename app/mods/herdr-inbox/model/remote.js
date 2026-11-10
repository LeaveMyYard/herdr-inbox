// A remote machine's rows. The model reads the local mirror only. One remote question at a time, no
// tabs: an answer goes under the identity the question was shown with, never whatever the mirror
// holds under that number now.

import { armedAt, backToList, mount, showList } from './groups.js';
import { copied, keyOf, kindOf, machineName, markBusy, problemKey, question, remoteAlert, remoteAlertKey, remoteGroup, remoteRef, remoteRefusal } from './data.js';
import { replaceWithPlaceholder } from './poll.js';
import { questionView } from './question.js';

export function openRemote(p, entry) {
  const kind = kindOf(entry);
  const remote = p.data.remote;
  if (kind === 'ralert') {
    const a = remoteAlert(remote, keyOf(entry));
    if (!a) return showList(p, entry);
    Object.assign(p, { mode: 'alert', entry, tabs: [], current: null });
    mount(p, { kind: 'alert', alert: a.alert.id, label: machineName(remote, a.machine_id), armed_at: armedAt(p) });
    p.remote_alert = copied(a);
  } else if (kind === 'routbox' || kind === 'rproblem') {
    p.notify('This is on a remote machine; d clears it here', 'information');
  } else if (kind === 'remote') {
    const g = remoteGroup(remote, keyOf(entry));
    if (!g) return showList(p, entry);
    const q = copied(g.questions[0]);
    Object.assign(p, { mode: 'remote', entry, tabs: [], current: null });
    const view = questionView(q.question, false, true, armedAt(p));
    p.remote_open = { label: machineName(remote, g.machine_id), question: q };
    mount(p, view);
  }
}

export function answerRemote(p, qid, detail) {
  const shown = p.remote_open;
  if (!shown || !detail || qid !== shown.question.question.id) return;
  const ref = remoteRef(shown.question);
  const why = remoteRefusal(p.data.remote, ref);
  if (why) return p.notify(`not queued: ${why}`, 'warning');
  p.checkpoint();
  markBusy(p.data.remote, ref);
  p.fx.push({ remote_answer: { ...ref, detail } });
  p.notify(`${shown.label}:Q${qid} answered · sending to ${shown.label}`, 'information');
  if (p.entry !== null && kindOf(p.entry) === 'remote' && !remoteGroup(p.data.remote, keyOf(p.entry))) {
    p.closed.set(p.entry, `answered · sending to ${shown.label}`);
  }
  backToList(p, false);
}

// "Chat about this": its pane id means nothing on this machine, so nothing is focused.
export function chatRemote(p) {
  if (p.remote_open) p.notify(`That agent runs on ${p.remote_open.label}: open it from herdr's machine view`, 'information');
}

// The question on screen closed over there: an inert placeholder, as a closed local tab.
export function pollRemote(p) {
  if (p.entry === null || kindOf(p.entry) !== 'remote' || !p.remote_open) return;
  const id = p.remote_open.question.question.id;
  const g = remoteGroup(p.data.remote, keyOf(p.entry));
  if (!g?.questions.some(q => q.question.id === id)) {
    replaceWithPlaceholder(p, `${p.remote_open.label}:Q${id}`, 'closed on the remote machine');
  }
}

// The question a question view shows: in remote mode the remote one, whose id a local one may share.
export function viewQuestion(p, qid) {
  if (p.mode !== 'remote') return question(p.data, qid);
  return p.remote_open?.question.question.id === qid ? p.remote_open.question.question : undefined;
}

export function ackRemote(p, entry) {
  const key = keyOf(entry);
  const a = remoteAlert(p.data.remote, key);
  if (a) {
    p.checkpoint();
    p.fx.push({ remote_ack: { machine_id: a.machine_id, alert_id: a.alert.id, db_id: a.db_id, akey: a.akey } });
    p.data.remote.alerts = p.data.remote.alerts.filter(x => remoteAlertKey(x) !== key);
  }
  p.closed.set(entry, 'acknowledged');
}

// `d` on a remote row that is no placeholder.
export function dismissRemote(p, entry) {
  const kind = kindOf(entry);
  const key = keyOf(entry);
  const remote = p.data.remote;
  if (kind === 'ralert') ackRemote(p, entry);
  else if (kind === 'routbox') {
    const row = remote.outbox.find(r => r.request_id === key && r.clearable);
    if (!row) return; // still queued, or being sent right now: nothing to clear
    p.checkpoint();
    p.fx.push({ clear_outbox: { request_id: key, expire: row.state === 'queued' } });
    remote.outbox = remote.outbox.filter(r => r.request_id !== key);
    p.closed.set(entry, 'cleared');
  } else if (kind === 'rproblem') {
    const bar = key.indexOf('|');
    const id = bar < 0 ? '' : key.slice(bar + 1);
    if (!/^-?\d+$/.test(id)) return;
    p.checkpoint();
    p.fx.push({ clear_problem: { machine_id: key.slice(0, bar), remote_id: Number(id) } });
    remote.problems = remote.problems.filter(x => problemKey(x) !== key);
    p.closed.set(entry, 'cleared');
  } else if (kind === 'remote') {
    const g = remoteGroup(remote, key);
    if (!g) return;
    const label = machineName(remote, g.machine_id);
    const questions = copied(g.questions);
    p.confirm = { what: `${label}:Q${questions[0].question.id}`, target: { kind: 'remote', entry, label, questions } };
  }
}

// `y` on a remote group: one dismissal per question, under the identity each had when `d` was pressed.
export function confirmedRemote(p, { entry, label, questions }) {
  p.checkpoint();
  for (const q of questions) {
    const ref = remoteRef(q);
    const why = remoteRefusal(p.data.remote, ref);
    if (why) return p.notify(`not queued: ${why}`, 'warning');
    markBusy(p.data.remote, ref);
    p.fx.push({ remote_dismiss: ref });
  }
  p.closed.set(entry, `dismissing on ${label}`);
}
