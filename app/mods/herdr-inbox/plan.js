// What the model's effects mean as commands. Pure: the book says which row a question is in.

const REMOTE = ['remote_answer', 'remote_dismiss', 'remote_ack', 'clear_outbox', 'clear_problem'];

function sendCommand(send, entryOf) {
  if (send.kind === 'retry') return { verb: 'retry', entry: `failed:${send.qid}`, sent: true, wanted: 1 };
  if (send.kind === 'submit') return { verb: 'answer', entry: entryOf(send.qids[0]), qids: send.qids, sent: true, wanted: send.qids.length };
  return { verb: 'answer', entry: entryOf(send.qid), qids: [send.qid], detail: send.detail, sent: true, wanted: 1 };
}

// The group's every question dismissed at once is one verb: all of them or none.
function oneDismissal(commands, book) {
  const entry = commands[0]?.entry;
  const row = entry ? book.row(entry) : null;
  const all = commands.length >= 2 && commands.every(c => c.verb === 'dismiss' && c.entry === entry);
  return all && row?.questions.length === commands.length ? [{ verb: 'dismiss', entry }] : commands;
}

// { commands, notices, close, nothing, refusedRemote }. `nothing`: a lone question with no answer, so
// no verb runs. The model's "Sending…" before a send is left out: it can't yet know that it will be.
export function plan(fx, book) {
  const entryOf = qid => book.rowOf(qid)?.key ?? null;
  const out = { commands: [], notices: [], close: false, nothing: false, refusedRemote: fx.some(e => REMOTE.some(name => name in e)) };
  fx.forEach((e, i) => {
    if (e.notify) {
      if (!fx[i + 1]?.send) out.notices.push(e.notify);
    } else if (e.draft) {
      const { qid, detail } = e.draft;
      out.commands.push({ verb: 'draft', entry: entryOf(qid), qids: [qid], detail, clear: detail === null });
    } else if (e.send?.kind === 'answer' && e.send.detail === null) out.nothing = true;
    else if (e.send) out.commands.push(sendCommand(e.send, entryOf));
    else if (e.dismiss) out.commands.push({ verb: 'dismiss', entry: entryOf(e.dismiss.qid), qids: [e.dismiss.qid] });
    else if (e.hide) out.commands.push({ verb: 'stop-showing', entry: `failed:${e.hide.qid}` });
    else if (e.ack) out.commands.push({ verb: 'ack', entry: `alert:${e.ack.alert}` });
    else if (e.go_to) out.commands.push({ go: e.go_to });
    else if (e.exit || e.quit) out.close = true;
  });
  out.commands = oneDismissal(out.commands, book);
  return out;
}
