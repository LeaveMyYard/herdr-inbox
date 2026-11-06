// Builds `herdr-inbox app <verb> --json` and reads what comes back (docs/protocol/app-commands-v1.md).
// A token is opaque: this file only echoes it.

const MAX_DETAIL = 65536; // bytes, from the contract: more is usage, exit 2
const CODES = new Set(['stale', 'already_sending', 'invalid', 'read_only', 'busy', 'not_migrated', 'error']);

const isId = value => Number.isSafeInteger(value) && value >= 1;

function bytes(text) {
  let n = 0;
  for (const c of text) {
    const code = c.codePointAt(0);
    n += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  }
  return n;
}

export function detailArgument(detail) {
  const text = JSON.stringify({ choices: detail.choices, notes: detail.notes, text: detail.text });
  return bytes(text) > MAX_DETAIL ? null : text;
}

const questions = ids => ids.flatMap(id => ['--question', String(id)]);

// The arguments after the launcher's `cli`, or null when these can't be a command.
export function verbArgs({ verb, entry, qids = [], detail, clear = false }, token) {
  if (typeof token !== 'string' || token === '' || !qids.every(isId)) return null;
  const head = ['app', verb, '--entry', entry, '--token', token, ...questions(qids)];
  if (clear) return [...head, '--clear', '--json'];
  if (!detail) return [...head, '--json'];
  const text = detailArgument(detail);
  return text === null ? null : [...head, '--detail', text, '--json'];
}

export const showArgs = qid => ['app', 'show', '--question', String(qid), '--json'];

function parsed(text) {
  try {
    const value = JSON.parse(text);
    return typeof value === 'object' && value !== null && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

const unknown = why => ({ kind: 'unknown', why });
const isIdList = value => Array.isArray(value) && value.every(Number.isSafeInteger);

function success(value, verb) {
  if (verb === 'show') {
    const row = value.row;
    return typeof row === 'object' && row !== null && row.key === value.entry ? { kind: 'shown', row } : unknown('a result with no row');
  }
  const { token, done, skipped, delivery } = value;
  if (!(token === null || typeof token === 'string') || !isIdList(done) || !isIdList(skipped)) return unknown('a result it could not read');
  if (!(delivery === null || delivery === 'queued' || delivery === 'held')) return unknown('a delivery word it does not know');
  return { kind: 'done', token, done, skipped, delivery };
}

// What a finished command said. Anything but a result of this shape is of unknown effect: the mod says
// so, reloads, and does not repeat the verb. `entry` is null for `show`, which names its own row.
export function readResult(result, verb, entry) {
  if (result === null) return unknown('it could not be started');
  if (result.code === 2) return unknown(result.stderr?.trim() || 'it refused its arguments');
  if (result.code !== 0 && result.code !== 1) return unknown(`it exited ${result.code}`);
  const value = parsed(result.stdout);
  if (value === null || value.v !== 1 || value.verb !== verb) return unknown('its output was not a result');
  if (entry !== null && value.entry !== entry) return unknown('its result named another row');
  if (result.code === 0 && value.ok === true) return success(value, verb);
  if (result.code === 1 && value.ok === false) {
    return {
      kind: 'refused', code: CODES.has(value.code) ? value.code : 'error', closed: typeof value.closed === 'string' ? value.closed : null,
      message: typeof value.message === 'string' ? value.message : '',
    };
  }
  return unknown('its exit status and its result disagree');
}
