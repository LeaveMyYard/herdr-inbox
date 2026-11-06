// The inbox overlay. So far it only says whether the inbox can be reached.

import { View, div } from 'gpui-kit';
import { v_flex } from 'gpui-base';
import { appHome, exec, home, readTextFile, theme } from 'herdr';
import { availability } from './plugin.js';

const WHY = {
  'not-installed': 'The install record is missing or unreadable. Run scripts/app-install from the plugin.',
  'no-plugin': 'The inbox plugin is not where it was installed. Run scripts/app-install from the plugin.',
  'no-app': "The plugin has no app commands: it runs Python, it has no Rust binary, or that binary is older than this mod.",
};

const TEXT = {
  checking: { title: 'Inbox', detail: 'Looking for the inbox…' },
  ready: { title: 'Inbox', detail: 'The inbox can be reached. Its questions will show here in a later version.' },
};

export function words(status) {
  if (status.state !== 'unavailable') return TEXT[status.state];
  return { title: 'Inbox unavailable', detail: WHY[status.reason] };
}

export default class HerdrInbox extends View {
  init(_props, cx) {
    this.status = { state: 'checking' };
    cx.spawn(async cx => {
      this.status = await availability({ appHome, exec, home, readTextFile });
      cx.notify();
    });
  }

  render() {
    const t = theme();
    const { title, detail } = words(this.status);
    return v_flex().id('herdr-inbox').size_full().p(18).gap(12).bg(t.popover).text_size(13)
      .child(div().text_size(15).font_semibold().text_color(t.foreground).child(title))
      .child(div().text_color(t.muted).child(detail));
  }
}
