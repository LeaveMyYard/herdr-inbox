// What the operator's actions do: answers, drafts, sends, hides, acknowledgements, dismissals and
// going to an agent. Each emits its effect and keeps the model's copy of the data as the write leaves
// it; the next snapshot confirms it.

import { GO_PAUSE, agentGone, alert, idOf, isAlert, isRemote, kindOf, openGroup, question, tabId } from './data.js';
import { back, backToList, navigate, openEntry, openQids } from './groups.js';
import { listTarget } from './keys.js';
import { ackRemote, answerRemote, chatRemote, confirmedRemote, dismissRemote, viewQuestion } from './remote.js';
import { goneBatchLive } from './rows.js';

export function writeDraftQuiet(p, qid, detail) {
  const q = question(p.data, qid);
  if (q?.status !== 'open') return false;
  q.draft = detail;
  return true;
}

export function writeDraft(p, qid, detail) {
  p.fx.push({ draft: { qid, detail } });
  return writeDraftQuiet(p, qid, detail);
}

function writeStatus(p, qid, status) {
  const q = question(p.data, qid);
  if (q?.status !== 'open') return false;
  q.status = status;
  p.data.open = p.data.open.filter(id => id !== qid);
  return true;
}

function closeQuestion(p, qid) {
  p.fx.push({ dismiss: { qid } });
  return writeStatus(p, qid, 'dismissed');
}

function send(p, what) {
  const qids = what.kind === 'submit' ? what.qids : [what.qid];
  p.notify(`Sending ${qids.map(q => `Q${q}`).join(', ')}…`, 'information');
  if (what.kind === 'answer' && what.detail && writeDraftQuiet(p, what.qid, what.detail)) writeStatus(p, what.qid, 'answered');
  if (what.kind === 'submit') {
    for (const qid of what.qids) {
      const q = question(p.data, qid);
      if (q?.status === 'open' && q.draft) writeStatus(p, qid, 'answered');
    }
  }
  p.fx.push({ send: what });
}

function afterSend(p, entry, whole) {
  if (entry !== null && whole) p.closed.set(entry, 'sending…');
  backToList(p, false);
}

function hide(p, qid) {
  p.fx.push({ hide: { qid } });
  p.data.failed = p.data.failed.filter(q => q !== qid);
}

function ack(p, entry) {
  if (kindOf(entry) === 'ralert') return ackRemote(p, entry);
  if (kindOf(entry) !== 'alert') return;
  const id = idOf(entry);
  p.fx.push({ ack: { alert: id } });
  const a = alert(p.data, id);
  if (a) a.status = 'acked';
  p.data.open_alerts = p.data.open_alerts.filter(x => x !== id);
  p.closed.set(entry, 'acknowledged');
}

function answered(p, qid, detail) {
  if (p.mode === 'remote') return answerRemote(p, qid, detail);
  const tab = `q:${qid}`;
  if (p.mode !== 'group' || p.gone.has(tab) || p.current !== tab) return; // never for a question not on screen
  if (p.single) {
    const { entry } = p;
    afterSend(p, entry, true);
    return send(p, { kind: 'answer', qid, detail });
  }
  if (!writeDraft(p, qid, detail)) p.notify(`Q${qid} is no longer open`, 'warning');
  if (detail) navigate(p, 1, null); // the last question moves on to Submit
}

export function message(p, msg) {
  const { entry } = p;
  switch (msg.t) {
    case 'back': return back(p);
    case 'quit': return p.quit();
    case 'open': return openEntry(p, msg.entry, null);
    case 'answered': return answered(p, msg.qid, msg.detail);
    case 'chat':
      if (p.mode === 'remote') return chatRemote(p);
      return focusPane(p, viewQuestion(p, msg.qid)?.pane_id, false);
    case 'edit': return navigate(p, 0, `q:${msg.qid}`);
    case 'submit': {
      const whole = openQids(p).every(q => msg.qids.includes(q));
      send(p, { kind: 'submit', qids: msg.qids });
      return afterSend(p, entry, whole);
    }
    case 'retry':
      send(p, { kind: 'retry', qid: msg.qid });
      return afterSend(p, entry, true);
    case 'hide':
      hide(p, msg.qid);
      p.closed.set(`failed:${msg.qid}`, 'hidden');
      return backToList(p, false);
    case 'ack':
      if (entry === null || p.mode !== 'alert' || p.closed.has(entry)) return;
      ack(p, entry);
      return backToList(p, true);
    default: return;
  }
}

function dismissRow(p, entry) {
  const kind = kindOf(entry);
  if (kind === 'alert') ack(p, entry);
  else if (kind === 'gone') {
    for (const qid of goneBatchLive(p)) hide(p, qid);
    p.closed.set(entry, 'hidden');
  } else if (kind === 'failed') {
    const q = question(p.data, idOf(entry));
    if (q && agentGone(q)) {
      hide(p, idOf(entry));
      p.closed.set(entry, 'hidden');
    }
  } else if (kind === 'group') {
    const qs = openGroup(p.data, idOf(entry)).map(q => q.id);
    if (qs.length === 0) return;
    const what = qs.length === 1 ? `Q${qs[0]}` : `Q${qs[0]}–Q${qs.at(-1)} (${qs.length} questions)`;
    p.confirm = { what, target: { kind: 'group', entry, qids: qs } };
  } else {
    dismissRemote(p, entry);
  }
}

// `d`: on a list row, acknowledge an alert, hide gone-agent answers, or ask to dismiss a whole group;
// inside a group, ask to dismiss the question on screen.
export function dismiss(p) {
  if (p.mode === 'list') {
    const entry = listTarget(p);
    if (entry !== null && !p.closed.has(entry)) dismissRow(p, entry);
    return;
  }
  if (p.mode !== 'group' || p.current === null || p.current === 'submit' || p.gone.has(p.current)) return;
  const qid = tabId(p.current);
  if (question(p.data, qid)) p.confirm = { what: `Q${qid}`, target: { kind: 'question', qid } };
}

// A row's pane, or undefined when the row has none to name. The gone summary and every remote row
// have none: a remote pane is on another machine, and its number can be a local alert's or question's.
function paneOf(p, entry) {
  const kind = kindOf(entry);
  if (kind === 'alert') return alert(p.data, idOf(entry))?.pane_id;
  if (kind === 'failed') return question(p.data, idOf(entry))?.pane_id;
  if (kind === 'group') return openGroup(p.data, idOf(entry))[0]?.pane_id;
  return undefined;
}

// `g`.
export function goTo(p) {
  if (p.mode === 'list') {
    const cursor = listTarget(p);
    if (cursor !== null && isRemote(cursor) && kindOf(cursor) !== 'rproblem') {
      return p.notify("That agent runs on a remote machine: open it from herdr's machine view", 'information');
    }
    if (cursor !== null) focusPane(p, paneOf(p, cursor), isAlert(cursor));
  } else if ((p.mode === 'failed' || p.mode === 'alert') && p.entry !== null) {
    focusPane(p, paneOf(p, p.entry), isAlert(p.entry));
  } else if (p.current?.startsWith('q:')) {
    focusPane(p, question(p.data, tabId(p.current))?.pane_id, false);
  }
}

// An alert's agent needs a deliberate `g`, after a pause.
function focusPane(p, pane, isAlertRow) {
  if (pane === undefined) return;
  if (!pane) return p.notify('could not focus: no pane', 'warning');
  const since = t => t === null || p.now >= t + GO_PAUSE;
  const graced = since(p.timers.prev_key_at) && since(p.timers.view_at);
  if (isAlertRow && !graced) return p.notify('Pause, then press g to go to that agent', 'information');
  p.fx.push({ go_to: { pane, graced } });
}

// The confirmation closed: only `y` confirms.
export function confirmed(p, target, yes) {
  if (!yes) return;
  if (target.kind === 'question') {
    if (closeQuestion(p, target.qid)) {
      p.notify(`Q${target.qid} dismissed`, 'information');
      p.gone.set(`q:${target.qid}`, 'dismissed');
      navigate(p, 1, null);
    }
  } else if (target.kind === 'group') {
    const closed = target.qids.filter(q => closeQuestion(p, q)).map(q => `Q${q}`);
    if (closed.length > 0) p.notify(`${closed.join(', ')} dismissed`, 'information');
    p.closed.set(target.entry, 'dismissed');
  } else {
    confirmedRemote(p, target);
  }
}
