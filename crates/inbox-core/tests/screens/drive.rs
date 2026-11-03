//! The popup driven as the capture's `pilot` drives Python's: opened on a
//! snapshot, keys pressed (each well after the last, so every view is
//! armed, as the capture runs with the arming delays at 0), polls, and the
//! screen rendered at a size. A frame is drawn after every key and poll, as
//! Textual draws one: the scroll offsets follow the cursor key by key.

use inbox_core::popup::{Event, Key, Open, Popup, Snapshot};
use inbox_core::popup_view::{Ctx, ViewState, render};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::data::HELD;

/// Milliseconds between two of the pilot's keys: past every arming delay.
pub const STEP: u64 = 2_000;

pub struct Pilot {
    pub popup: Popup,
    pub state: ViewState,
    pub now: u64,
    pub size: (u16, u16),
}

impl Pilot {
    /// Open on `snapshot` with the focus in pane w1:p1.
    pub fn open(snapshot: Snapshot, size: (u16, u16)) -> Pilot {
        let now = 1_000_000;
        let open = Open {
            focus_pane: Some("w1:p1".into()),
            snapshot,
            deeplink: None,
        };
        let (popup, _) = Popup::open(open, now);
        Pilot {
            popup,
            state: ViewState::default(),
            now,
            size,
        }
    }

    /// Press keys by Textual name (`"down"`, `"n"`), one after another.
    pub fn press(&mut self, keys: &[&str]) {
        for k in keys {
            self.now += STEP;
            self.popup.handle(Event::Key(Key::parse(k)), self.now);
            self.screen();
        }
    }

    /// Type `text` a character at a time (`" "` is `space`).
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let name = if c == ' ' {
                "space".to_string()
            } else {
                c.to_string()
            };
            self.press(&[name.as_str()]);
        }
    }

    pub fn poll(&mut self, snapshot: Snapshot) {
        self.now += STEP;
        self.popup.handle(Event::Poll(snapshot), self.now);
        self.screen();
    }

    /// The screen now (wall clock held at `HELD`), as the terminal gets it.
    pub fn screen(&mut self) -> Buffer {
        self.now += STEP;
        let ctx = Ctx {
            now: self.now,
            wall: HELD,
            ..Ctx::default()
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, self.size.0, self.size.1));
        render(&self.popup, &ctx, &mut self.state, &mut buf);
        buf
    }
}
