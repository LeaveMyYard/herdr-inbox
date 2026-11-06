// Reads the feed (docs/protocol/app-feed-v1.md). A read error never becomes an empty inbox: anything
// this file does not accept leaves the last good feed in place.

const STALE_AFTER = 15;

const isId = value => Number.isSafeInteger(value);
const isText = value => typeof value === 'string';
const nullable = (value, check) => value === null || check(value);
const isObject = value => typeof value === 'object' && value !== null && !Array.isArray(value);
const LOCAL = new Set(['group', 'failed', 'alert']);
const KNOWN = new Set([...LOCAL, 'remote_group', 'remote_outbox', 'remote_problem', 'remote_alert']);

function goodDetail(d) {
  return isObject(d) && Array.isArray(d.choices) && d.choices.every(isText) && isObject(d.notes)
    && Object.values(d.notes).every(isText) && nullable(d.text, isText);
}

function goodQuestion(q) {
  return isObject(q) && isId(q.id) && isText(q.question) && typeof q.multi_select === 'boolean' && typeof q.cut === 'boolean'
    && nullable(q.header, isText) && nullable(q.context, isText) && nullable(q.draft, goodDetail) && Array.isArray(q.options)
    && q.options.every(o => isObject(o) && isText(o.label) && nullable(o.preview, isText) && nullable(o.description, isText));
}

const goodAsker = a => isObject(a) && isText(a.host);
const goodMachine = m => isObject(m) && isText(m.id);

// The fields a decision reads, and that the key says what the fields say: a verb's `--entry` and
// `--question` come from here, so a row whose name and contents disagree is no row to act on.
function goodEntry(e) {
  if (!isObject(e) || !isText(e.key) || !isText(e.kind) || !Array.isArray(e.actions) || !e.actions.every(isText)) return false;
  if (e.actions.length > 0 ? !isText(e.token) : e.token !== null) return false;
  switch (e.kind) {
    case 'group':
      return isId(e.group) && e.key === `group:${e.group}` && goodAsker(e.asker) && Array.isArray(e.questions)
        && e.questions.length > 0 && e.questions.every(goodQuestion);
    case 'failed':
      return isId(e.question_id) && isId(e.group) && e.key === `failed:${e.question_id}` && isText(e.question) && goodAsker(e.asker)
        && isObject(e.delivery) && isText(e.delivery.state) && typeof e.cut === 'boolean';
    case 'alert':
      return isId(e.id) && e.key === `alert:${e.id}` && isText(e.title) && goodAsker(e.asker);
    case 'remote_group':
      return goodMachine(e.machine) && e.key.startsWith(`remote:${e.machine.id}|`) && Array.isArray(e.questions)
        && e.questions.length > 0 && e.questions.every(goodQuestion);
    case 'remote_outbox':
      return goodMachine(e.machine) && e.key.startsWith('routbox:') && isId(e.remote_id) && isText(e.state);
    case 'remote_problem':
      return goodMachine(e.machine) && isId(e.remote_id) && e.key === `rproblem:${e.machine.id}|${e.remote_id}` && isText(e.question);
    case 'remote_alert':
      return goodMachine(e.machine) && isId(e.id) && e.key === `ralert:${e.machine.id}|${e.id}` && isText(e.title);
    default:
      return !KNOWN.has(e.kind); // an unknown kind stays visible, with nothing offered on it
  }
}

function goodOmitted(o) {
  return isObject(o) && ['groups', 'questions', 'failed', 'alerts', 'remote'].every(k => isId(o[k]) && o[k] >= 0)
    && Array.isArray(o.question_ids) && o.question_ids.every(isId);
}

// { feed } when this is a v1 feed that can be used, else { problem, errors }.
export function parseFeed(text) {
  if (text === null) return { problem: 'missing', errors: [] };
  let feed;
  try {
    feed = JSON.parse(text);
  } catch {
    return { problem: 'unreadable', errors: [] };
  }
  if (!isObject(feed) || feed.v !== 1) return { problem: 'unsupported', errors: [] };
  const errors = Array.isArray(feed.errors) ? feed.errors.filter(isText) : [];
  if (feed.ok === false) return { problem: 'not-ok', errors };
  if (feed.ok !== true || !goodOmitted(feed.omitted) || !Array.isArray(feed.entries) || !feed.entries.every(goodEntry)) {
    return { problem: 'unreadable', errors };
  }
  const keys = feed.entries.map(e => e.key);
  if (new Set(keys).size !== keys.length) return { problem: 'unreadable', errors };
  return { feed: { ...feed, errors } };
}

// A view rule, not proof the writer died: a stale feed stays on screen, and tokens still protect it.
export function isStale(generatedAt, nowSeconds) {
  return typeof generatedAt !== 'number' || !Number.isFinite(generatedAt) || generatedAt > nowSeconds || nowSeconds - generatedAt > STALE_AFTER;
}

export const leftOut = feed => feed.omitted.groups + feed.omitted.failed + feed.omitted.alerts + feed.omitted.remote + feed.omitted.questions;

// What the operator must be told, or null: the required rule for an incomplete feed.
export function incompleteness(feed) {
  if (feed.errors.length === 0 && leftOut(feed) === 0) return null;
  const { groups, questions, failed, alerts, remote, question_ids: ids } = feed.omitted;
  const say = (n, one, many) => (n > 0 ? [`${n} ${n === 1 ? one : many}`] : []);
  const counts = [
    ...say(questions, 'question', 'questions'), ...say(groups, 'group', 'groups'), ...say(failed, 'undelivered answer', 'undelivered answers'),
    ...say(alerts, 'alert', 'alerts'), ...say(remote, 'remote row', 'remote rows'),
  ];
  return { counts, errors: feed.errors, ids, unlisted: Math.max(questions - ids.length, 0) };
}

// Whether a row that is not in this feed can be taken as closed. A row left out to fit the size limit
// is still open; past the 1000 ids, or for kinds with no ids, nothing says which rows were left out.
export function absentMeansClosed(row, feed) {
  const { omitted } = feed;
  switch (row.kind) {
    case 'group':
      if (omitted.groups === 0) return true;
      if (omitted.question_ids.length < omitted.questions) return false;
      return !row.questions.some(q => omitted.question_ids.includes(q.id));
    case 'failed': return omitted.failed === 0;
    case 'alert': return omitted.alerts === 0;
    default: return omitted.remote === 0;
  }
}
