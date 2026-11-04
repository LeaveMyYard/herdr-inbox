//! The popup's state ([`Popup`]), how events enter it, and what the
//! renderer may read.

use std::collections::{HashMap, VecDeque};

use super::data::{Detail, Millis, Snapshot};
use super::entry::{Entry, Mode, Tab};
use super::event::{Effect, Event, Open, Severity};
use super::focus::Focus;
use super::ordered::Ordered;
use super::remote::{RemoteAlert, RemoteQuestion};
use super::views::View;

pub use super::confirm::Confirm;

/// The inbox popup (Python `InboxApp`). Build it with [`Popup::open`], feed
/// it [`Popup::handle`], carry out the effects both return.
#[derive(Clone, Debug)]
pub struct Popup {
    pub(super) data: Snapshot,
    pub(super) focus_pane: Option<String>,
    pub(super) mode: Mode,
    pub(super) entries: Vec<Entry>,
    pub(super) closed: Ordered<Entry, String>,
    pub(super) arrived_at: HashMap<Entry, Millis>,
    pub(super) list_cursor: Option<Entry>,
    pub(super) direct: bool,
    pub(super) held: bool,
    pub(super) held_at: Millis,
    pub(super) entry: Option<Entry>,
    pub(super) tabs: Vec<Tab>,
    pub(super) current: Option<Tab>,
    pub(super) gone: Ordered<Tab, String>,
    pub(super) single: bool,
    pub(super) gone_batch: Option<Vec<i64>>,
    pub(super) timers: Timers,
    pub(super) view: View,
    pub(super) focus: Focus,
    pub(super) confirm: Option<Confirm>,
    pub(super) remote_open: Option<Shown>,
    pub(super) remote_alert: Option<RemoteAlert>,
    pub(super) running: bool,
    pub(super) now: Millis,
    pub(super) fx: Vec<Effect>,
    pub(super) queue: VecDeque<Msg>,
}

/// The remote question on screen (Python `remote_open`): what an answer is
/// queued under, and its machine's label as it was then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub label: Option<String>,
    pub question: RemoteQuestion,
}

/// The clock readings the key rules use (`None`: never).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timers {
    /// Keys before this are dropped ([`GRACE`](super::GRACE)).
    pub grace_until: Option<Millis>,
    /// The last view change.
    pub view_at: Option<Millis>,
    /// The last key.
    pub last_key_at: Option<Millis>,
    /// The key before it.
    pub prev_key_at: Option<Millis>,
}

/// What a handler leaves for after it (Python's `post_message` and
/// `call_next`, in order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Msg {
    Back,
    Quit,
    Answered(i64, Option<Detail>),
    Chat(i64),
    Edit(i64),
    Submit(Vec<i64>),
    Retry(i64),
    Hide(i64),
    Ack,
    Open(Entry),
}

impl Popup {
    /// Open the popup (Python `InboxApp.on_mount`).
    pub fn open(open: Open, now: Millis) -> (Popup, Vec<Effect>) {
        let mut p = Popup {
            data: open.snapshot,
            focus_pane: open.focus_pane,
            mode: Mode::Empty,
            entries: Vec::new(),
            closed: Ordered::default(),
            arrived_at: HashMap::new(),
            list_cursor: None,
            direct: false,
            held: false,
            held_at: 0,
            entry: None,
            tabs: Vec::new(),
            current: None,
            gone: Ordered::default(),
            single: false,
            gone_batch: None,
            timers: Timers::default(),
            view: View::None,
            focus: Focus::View,
            confirm: None,
            remote_open: None,
            remote_alert: None,
            running: true,
            now,
            fx: Vec::new(),
            queue: VecDeque::new(),
        };
        let focus = p.focus_pane.clone();
        p.entries = p.current_entries(focus.as_deref());
        let link = open.deeplink;
        let target = link
            .as_ref()
            .and_then(|l| l.target.clone().map(|t| (t, l.qid)));
        if let Some(l) = link.as_ref().filter(|l| l.target.is_none()) {
            p.notify(&l.missing, Severity::Information);
        }
        match target.filter(|(t, _)| p.entries.contains(t)) {
            Some((t, qid)) => {
                p.held = link.is_some_and(|l| l.hold);
                p.held_at = now;
                p.open_entry(&t, qid);
            }
            None if p.entries.is_empty() => p.show_empty(),
            None => {
                let questions: Vec<&Entry> = p.entries.iter().filter(|e| !e.is_alert()).collect();
                if let [only @ Entry::Group(_)] = questions[..] {
                    let only = only.clone();
                    p.direct = true;
                    p.open_entry(&only, None);
                } else {
                    let first = questions.first().map(|e| (*e).clone());
                    p.show_list(first);
                }
            }
        }
        p.drain();
        let fx = std::mem::take(&mut p.fx);
        (p, fx)
    }

    /// Handle one event at `now`; the effects to carry out, in order.
    pub fn handle(&mut self, event: Event, now: Millis) -> Vec<Effect> {
        self.now = now;
        if self.running {
            match event {
                Event::Key(key) => self.key(&key),
                Event::Poll(snapshot) => self.poll(snapshot),
                Event::Sent {
                    done,
                    wanted,
                    snapshot,
                } => self.sent(&done, wanted, snapshot),
                Event::Focused { pane, error } => self.focused(&pane, error.as_deref()),
                Event::Data(snapshot) => self.data = snapshot,
            }
            self.drain();
        }
        std::mem::take(&mut self.fx)
    }

    /// Run what the handlers left for after them, in order.
    fn drain(&mut self) {
        while let Some(msg) = self.queue.pop_front() {
            if self.running {
                self.message(msg);
            }
        }
    }

    pub(super) fn notify(&mut self, text: &str, severity: Severity) {
        self.fx.push(Effect::Notify {
            text: text.to_string(),
            severity,
        });
    }

    pub(super) fn later(&mut self, msg: Msg) {
        self.queue.push_back(msg);
    }

    /// Close at once (Python `exit`).
    pub(super) fn exit(&mut self) {
        self.fx.push(Effect::Exit);
        self.running = false;
    }

    /// Close once in-flight sends are recorded (Python `action_quit`).
    pub(super) fn quit(&mut self) {
        self.fx.push(Effect::Quit);
        self.running = false;
    }

    // -- what the renderer reads ---------------------------------------------------

    /// The database as the model sees it (the last snapshot and the
    /// model's own writes since).
    pub fn data(&self) -> &Snapshot {
        &self.data
    }
    /// What the popup shows.
    pub fn mode(&self) -> Mode {
        self.mode
    }
    /// The list's rows, placeholders included, in order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// Rows whose group or answer ended, with why: inert placeholders.
    pub fn closed(&self) -> &Ordered<Entry, String> {
        &self.closed
    }
    /// When a row arrived in the background (it takes no Enter or digit
    /// until [`ROW_ARM`](super::ROW_ARM) after it).
    pub fn arrived_at(&self, entry: &Entry) -> Option<Millis> {
        self.arrived_at.get(entry).copied()
    }
    /// The list row the popup returns to.
    pub fn list_cursor(&self) -> Option<&Entry> {
        self.list_cursor.as_ref()
    }
    /// Opened straight into the only group: Esc closes the popup.
    pub fn direct(&self) -> bool {
        self.direct
    }
    /// A deep link waits for a deliberate Tab/→.
    pub fn held(&self) -> bool {
        self.held
    }
    /// The open group or ⚠/alert entry.
    pub fn entry(&self) -> Option<&Entry> {
        self.entry.as_ref()
    }
    /// The open group's question tabs (placeholders included; the Submit tab
    /// is [`Popup::all_tabs`]).
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }
    /// The tab on screen.
    pub fn current(&self) -> Option<Tab> {
        self.current
    }
    /// Tabs whose question closed, with why.
    pub fn gone(&self) -> &Ordered<Tab, String> {
        &self.gone
    }
    /// A lone question: Enter sends at once (fixed when the group opened).
    pub fn single(&self) -> bool {
        self.single
    }
    /// The clock readings the key rules use.
    pub fn timers(&self) -> Timers {
        self.timers
    }
    /// The view on screen.
    pub fn view(&self) -> &View {
        &self.view
    }
    /// The dismiss confirmation, if open.
    pub fn confirm(&self) -> Option<&Confirm> {
        self.confirm.as_ref()
    }
    /// Still open (no [`Effect::Exit`] or [`Effect::Quit`] yet).
    pub fn running(&self) -> bool {
        self.running
    }
    /// The answers of the gone-agents summary row still shown.
    pub fn gone_live(&self) -> Vec<i64> {
        self.gone_batch_live()
    }
}
