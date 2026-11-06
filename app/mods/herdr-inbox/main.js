// The inbox overlay: the host's side of `session.js`. It reads the feed, turns keys into model keys
// and runs the commands the session asks for. It writes nothing itself.

import { View } from 'gpui-kit';
import { agents, appHome, closeOverlay, exec, home, notify, props, readTextFile, run, settings, theme, watchFolder } from 'herdr';
import { isPaste, keyName, pastedKeys } from './keymap.js';
import { availability, ensure, launcher } from './plugin.js';
import { Session } from './session.js';
import { render } from './view.js';

const RELOAD_MS = 2000;
const ENSURE_MS = 5000;

const WHY = {
  'not-installed': 'The install record is missing or unreadable. Run scripts/app-install from the plugin.',
  'no-plugin': 'The inbox plugin is not where it was installed. Run scripts/app-install from the plugin.',
  'no-app': 'The plugin has no app commands: it runs Python, it has no Rust binary, or that binary is older than this mod.',
};

const TEXT = { checking: { title: 'Inbox', detail: 'Looking for the inbox…' } };

export function words(status) {
  if (status.state !== 'unavailable') return TEXT[status.state] ?? { title: 'Inbox', detail: 'Waiting for the inbox feed…' };
  return { title: 'Inbox unavailable', detail: WHY[status.reason] };
}

const isQuestionId = value => Number.isSafeInteger(value) && value >= 1;

// How the mod opens: props say a question, as Panorama and the toast do. A key opens it plain.
function linkFrom(p) {
  return { question: isQuestionId(p?.question) ? p.question : null, hold: p?.hold !== false };
}

export default class HerdrInbox extends View {
  init(_props, cx) {
    this.status = { state: 'checking' };
    this.session = new Session(linkFrom(props()));
    this.running = false;
    this.ensuredAt = 0;
    this.handle = cx.focus_handle();
    this.handle.focus();
    cx.spawn(async cx => {
      this.status = await availability({ appHome, exec, home, readTextFile });
      cx.notify();
      if (this.status.state !== 'ready') return;
      await this.ensureRuns();
      await this.reload(cx);
      cx.timer.every(RELOAD_MS, cx => this.tick(cx));
      this.watch(cx);
    });
  }

  feedDir() {
    return `${appHome()}/data/herdr-inbox`;
  }

  watch(cx) {
    cx.spawn(async cx => {
      for (;;) {
        await watchFolder(this.feedDir());
        await this.reload(cx);
      }
    });
  }

  async ensureRuns() {
    this.ensuredAt = Date.now();
    try {
      await ensure({ appHome, exec }, this.status.root);
    } catch {
      // the feed going stale says so: the slow timer asks again
    }
  }

  tick(cx) {
    cx.spawn(async cx => {
      if (this.session.stale && Date.now() - this.ensuredAt >= ENSURE_MS) await this.ensureRuns();
      await this.reload(cx);
    });
  }

  async reload(cx) {
    let text = null;
    try {
      text = await readTextFile(`${this.feedDir()}/feed.json`);
    } catch {
      text = null;
    }
    this.session.read(text, Date.now());
    await this.pump(cx);
  }

  // Runs what the session asks for, one command at a time, until it asks for nothing.
  async pump(cx) {
    if (this.running) return;
    this.running = true;
    try {
      for (let step = this.session.next(Date.now()); step; step = this.session.next(Date.now())) {
        cx.notify();
        this.session.finish(step, await this.carry(step), Date.now());
      }
    } finally {
      this.running = false;
    }
    this.settle();
    cx.notify();
    if (this.session.reload) {
      this.session.reload = false;
      await this.reload(cx);
    }
  }

  async carry(step) {
    if (step.go) return this.goTo(step.go);
    try {
      return await exec(...launcher(this.status.root, step.args));
    } catch {
      return null;
    }
  }

  // Going to an agent is no verb: the app's command for a core terminal, the herdr popup for a TUI one.
  async goTo({ pane, graced }) {
    const [host, ...rest] = pane.split(':');
    const target = rest.join(':');
    if (host === 'core') {
      const blocked = agents().find(a => a.address === target)?.status === 'blocked';
      if (blocked && !graced) return { notice: 'That agent is waiting at a prompt: pause, then press g' };
      run('agent.focus', { terminal: target });
      return {};
    }
    if (host !== 'tui') return { error: 'open it from the herdr TUI' };
    try {
      const { code } = await exec(...launcher(this.status.root, ['open', '--question', `Q${target}`]));
      if (code === 0) return {};
      return { error: code === 3 ? 'a herdr popup is open already' : 'no herdr server, or herdr refused' };
    } catch {
      return { error: 'herdr could not be started' };
    }
  }

  settle() {
    for (const { text, severity } of this.session.toasts.splice(0)) {
      notify({ title: 'Inbox', body: text, level: severity === 'warning' ? 'warning' : 'info' });
    }
    if (this.session.closing) closeOverlay(props().overlay);
  }

  press(name, cx) {
    this.session.key(name, Date.now());
    cx.spawn(cx => this.pump(cx));
    cx.notify();
  }

  onKey(e, cx) {
    cx.stop_propagation();
    const name = keyName(e);
    if (name === null) return;
    if (isPaste(name)) {
      this.session.paste(pastedKeys(cx.read_from_clipboard() ?? ''), Date.now());
      cx.notify();
      return;
    }
    this.press(name, cx);
  }

  wordsNow() {
    return words(this.status);
  }

  agentsNow() {
    return agents();
  }

  render() {
    return render(this, { ...theme(), mono: settings().font_family ?? 'monospace' });
  }
}
