// The real main.js in a fake host: keys reach `onKey`, clicks `onRow` and `onButton`, Esc `onEscape`,
// and every command goes through `exec` to the stand-in, as in a window. Only the host is a stand-in.
import { world as host } from './herdr.mjs';
import { StandIn } from './standin.mjs';

const ROOT = '/fake/plugin';
const NO_MODS = { shift: false, control: false, alt: false, platform: false, function: false };

export class Window {
  constructor(HerdrInbox, feed, { props = { overlay: 'o1' }, rows = [], standin = {}, agents = [] } = {}) {
    this.db = new StandIn(feed, { rows, ...standin });
    this.clock = 1767348100000;
    this.ran = [];
    this.tasks = [];
    this.timers = [];
    this.gate = null; // a promise a verb waits on, for a verb still running
    Date.now = () => this.clock;
    Object.assign(host, { toasts: [], closed: [], ran: [], props, agents, clipboard: '' });
    const appHome = host.appHome;
    host.files = {
      [`${appHome}/data/herdr-inbox/install.json`]: JSON.stringify({ v: 1, plugin_root: ROOT, plugin_moves: false, claude_config: null }),
      [`${ROOT}/herdr-plugin.toml`]: 'id = "leavemyyard.herdr-inbox"\n',
      [`${ROOT}/scripts/launch`]: '#\n',
    };
    Object.defineProperty(host.files, `${appHome}/data/herdr-inbox/feed.json`, { enumerable: true, get: () => this.feedText() });
    host.exec = (_command, args) => this.exec(args);
    this.cx = {
      spawn: task => { this.tasks.push(task); },
      notify() {},
      focus_handle: () => ({ focus() {} }),
      timer: { every: (ms, task) => this.timers.push({ ms, task }) },
      bind_keys: bindings => { this.bound = bindings; return bindings.length; },
      stop_propagation() {},
      read_from_clipboard: () => host.clipboard,
    };
    this.view = new HerdrInbox();
    this.view.init({}, this.cx);
  }

  feedText() {
    if (this.feedOverride !== undefined) return this.feedOverride;
    this.db.clock = this.clock / 1000 - 1;
    return this.db.published();
  }

  async exec(args) {
    const [, verb] = args;
    if (verb === 'which') return { code: 0, stdout: '/fake/bin\n', stderr: '' };
    const cli = args.slice(2);
    this.ran.push(cli);
    if (cli[0] === 'app' && cli[1] === '--help') return { code: 0, stdout: 'herdr-inbox app v1\n', stderr: '' };
    if (cli[0] === 'app-feed') this.readBeforeEnsure ??= this.view.session.popup !== null;
    if (cli[0] === 'app-feed' || cli[0] === 'worker') return { code: 0, stdout: '', stderr: '' };
    if (cli[0] === 'open') return { code: this.openCode ?? 0, stdout: '', stderr: '' };
    if (this.gate) await this.gate;
    return this.db.run(cli);
  }

  // Runs what was spawned until nothing moves. The feed watcher never ends, so a task gets moments.
  async settle() {
    for (let i = 0; i < 40; i += 1) {
      while (this.tasks.length > 0) {
        const done = this.tasks.shift()(this.cx);
        await Promise.race([done, moment()]);
      }
      await moment();
    }
  }

  async opened() {
    await this.settle();
    return this;
  }

  tick(ms = 700) {
    this.clock += ms;
  }

  // A key as the host gives it.
  event(key, { held = false, char, shift = false, control = false } = {}) {
    const typed = char === undefined ? (key.length === 1 && !control ? key : undefined) : char;
    return { key, key_char: typed, keystroke: key, modifiers: { ...NO_MODS, shift, control }, is_held: held };
  }

  async key(key, options = {}) {
    this.tick(options.gap ?? 700);
    this.view.onKey(this.event(key, options), this.cx);
    await this.settle();
  }

  // A key held down: the first press, then the host's repeats, the time of a long hold.
  async hold(key, ms = 2500) {
    await this.key(key);
    for (let at = 0; at < ms; at += 40) await this.key(key, { held: true, gap: 40 });
  }

  // Esc pressed and let go; `{ up: false }` keeps it down, as a held key does.
  async escape({ up = true } = {}) {
    this.tick();
    this.view.onEscape(this.cx);
    if (up) this.view.onKeyUp({ key: 'escape' });
    await this.settle();
  }

  async click(row) {
    this.tick();
    this.view.onRow(row, this.cx);
    await this.settle();
  }

  async button(name) {
    this.tick();
    this.view.onButton(name, this.cx);
    await this.settle();
  }

  async timer() {
    this.tick(2000);
    for (const { task } of this.timers) task(this.cx);
    await this.settle();
  }

  verbs(name) {
    return this.ran.filter(a => a[0] === 'app' && a[1] === name);
  }

  get popup() {
    return this.view.session.popup;
  }

  get toasts() {
    return host.toasts.map(t => t.body).join('\n');
  }

  get closed() {
    return host.closed.length > 0;
  }

  get host() {
    return host;
  }
}

const moment = () => new Promise(done => { setImmediate(done); });
