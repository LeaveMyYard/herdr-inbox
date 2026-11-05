//! The loop's test rig: a scripted terminal on a held clock, a fake
//! delivery hand-off and fake herdr agents, over a real temporary database.

use std::cell::Cell;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use inbox_core::popup::{DeepLink, Key, Millis, Open};
use inbox_core::popup_view::Theme;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use crate::store::question::QOption;
use crate::store::questions::NewQuestion;
use crate::store::{Question, Store};
use crate::tui::goto::Agents;
use crate::tui::marks::Marks;
use crate::tui::run::{Parts, run};
use crate::tui::screen::{Clock, Input, Screen};
use crate::tui::send::Sender;
use crate::tui::send::handoff::Handoff;
use crate::tui::snapshot::{Shown, read};

/// A deliberate key comes this long after the last thing (past every arming).
pub const GAP: Millis = 600;

/// One step of the script.
pub enum Step {
    /// A key, [`GAP`] after the last step.
    Key(Key),
    /// A key at once (inside the arming and grace windows).
    Now(Key),
    /// Another input, at once.
    Input(Input),
    /// The held clock moves on.
    Wait(Millis),
    /// Real time passes (a background thread's report arrives).
    Sleep(u64),
    /// Wait (real time, the clock moving slowly) until this holds.
    Until(Box<dyn Fn() -> bool>),
    /// Look at the screen as last drawn.
    Look(Box<dyn FnMut(&str)>),
    /// Do something (a write from another process, say).
    Do(Box<dyn FnOnce()>),
}

/// A key step for a Textual key name.
pub fn key(name: &str) -> Step {
    Step::Key(Key::parse(name))
}

/// The held clock.
#[derive(Default)]
pub struct HeldClock(Cell<Millis>);

impl Clock for HeldClock {
    fn now(&self) -> Millis {
        self.0.get()
    }
    fn wall(&self) -> f64 {
        1_800_000_000.0
    }
}

impl HeldClock {
    fn advance(&self, ms: Millis) {
        self.0.set(self.0.get() + ms);
    }
}

/// A terminal that plays a script.
pub struct Script {
    steps: VecDeque<Step>,
    clock: Rc<HeldClock>,
    /// The screen as last drawn, one line per row.
    pub last: String,
    /// When each frame was drawn (the held clock).
    pub frames: Vec<Millis>,
    /// How long the loop offered to wait at each call.
    pub waits: Vec<Duration>,
    started: Option<Instant>,
}

impl Screen for Script {
    fn next(&mut self, timeout: Duration) -> io::Result<Option<Input>> {
        self.waits.push(timeout);
        let Some(step) = self.steps.pop_front() else {
            std::thread::sleep(Duration::from_millis(2));
            self.clock.advance(50);
            return Ok(Some(Input::Closed));
        };
        Ok(match step {
            Step::Key(k) => {
                self.clock.advance(GAP);
                Some(Input::Key(k))
            }
            Step::Now(k) => {
                self.clock.advance(1);
                Some(Input::Key(k))
            }
            Step::Input(i) => Some(i),
            Step::Wait(ms) => {
                self.clock.advance(ms);
                None
            }
            Step::Sleep(ms) => {
                std::thread::sleep(Duration::from_millis(ms));
                None
            }
            Step::Until(cond) => {
                let started = *self.started.get_or_insert_with(Instant::now);
                if !cond() {
                    assert!(
                        started.elapsed() < Duration::from_secs(20),
                        "timed out waiting"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                    self.clock.advance(5);
                    self.steps.push_front(Step::Until(cond));
                } else {
                    self.started = None;
                }
                None
            }
            Step::Look(mut f) => {
                f(&self.last);
                None
            }
            Step::Do(f) => {
                f();
                None
            }
        })
    }

    fn draw(&mut self, paint: &mut dyn FnMut(&mut Buffer)) -> io::Result<()> {
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 30));
        paint(&mut buf);
        self.last = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.frames.push(self.clock.now());
        Ok(())
    }
}

/// The fake hand-off: what delivery was asked to do, in order. A closed
/// gate holds every delivery step until it opens.
#[derive(Default)]
pub struct FakeHandoff {
    pub calls: Mutex<Vec<String>>,
    gate: Mutex<bool>,
    open: Condvar,
}

impl FakeHandoff {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    pub fn close_gate(&self) {
        *self.gate.lock().unwrap() = true;
    }
    pub fn open_gate(&self) {
        *self.gate.lock().unwrap() = false;
        self.open.notify_all();
    }
    fn pass_gate(&self) {
        let mut shut = self.gate.lock().unwrap();
        while *shut {
            shut = self.open.wait(shut).unwrap();
        }
    }
    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }
}

impl Handoff for FakeHandoff {
    fn ensure_worker(&self) {
        self.log("ensure".into());
    }
    fn deliver_here(&self, _: &Store, qs: &[Question], force: bool) -> rusqlite::Result<()> {
        self.pass_gate();
        let ids: Vec<i64> = qs.iter().map(|q| q.id).collect();
        self.log(format!("here {ids:?} {force}"));
        Ok(())
    }
    fn spawn_delivery(&self, qids: &[i64], force: bool) {
        self.pass_gate();
        self.log(format!("spawn {qids:?} {force}"));
    }
}

/// Fake herdr agents: who was focused, from which thread.
#[derive(Default)]
pub struct FakeAgents {
    pub blocked: AtomicBool,
    pub error: Mutex<Option<String>>,
    pub focused: Mutex<Vec<(String, String)>>,
}

impl Agents for FakeAgents {
    fn blocked(&self, _: &str) -> bool {
        self.blocked.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn focus(&self, pane: &str) -> Option<String> {
        let thread = std::thread::current().name().unwrap_or("").to_string();
        self.focused
            .lock()
            .unwrap()
            .push((pane.to_string(), thread));
        self.error.lock().unwrap().clone()
    }
}

/// A database, its state directory and the fakes.
pub struct Rig {
    _dir: tempfile::TempDir,
    pub db: PathBuf,
    pub state: PathBuf,
    pub store: Store,
    pub handoff: Arc<FakeHandoff>,
    pub agents: Arc<FakeAgents>,
    pub marked: Arc<Mutex<Vec<Option<String>>>>,
    /// The popup's busy timeout.
    pub timeout: Duration,
}

/// What a run left.
pub struct Ran {
    /// The loop's waits.
    pub waits: Vec<Duration>,
}

impl Rig {
    pub fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("inbox.db");
        let store = Store::open_migrating(&db, Duration::from_secs(5)).unwrap();
        Rig {
            state: dir.path().join("state"),
            db,
            store,
            _dir: dir,
            handoff: Arc::default(),
            agents: Arc::default(),
            marked: Arc::default(),
            timeout: Duration::from_secs(5),
        }
    }

    /// Files a question from `pane` with `options`; its id.
    pub fn ask(&self, pane: &str, text: &str, options: &[&str]) -> i64 {
        self.ask_in(pane, text, options, None)
    }

    /// Files a question into `group` (`None`: a group of its own).
    pub fn ask_in(&self, pane: &str, text: &str, options: &[&str], group: Option<i64>) -> i64 {
        let opts: Vec<QOption> = options
            .iter()
            .map(|l| QOption {
                label: (*l).to_string(),
                ..QOption::default()
            })
            .collect();
        let q = NewQuestion {
            question: text,
            options: (!opts.is_empty()).then_some(opts),
            context: None,
            asked_by: None,
            pane_id: Some(pane),
            tab_id: None,
            tab_label: None,
            session_id: None,
            cwd: None,
            header: None,
            multi_select: false,
            group_id: group,
        };
        self.store.insert(&q).unwrap().id
    }

    pub fn get(&self, qid: i64) -> Question {
        self.store.get(qid).unwrap().unwrap()
    }

    /// Runs the popup over `steps` (the operator's pane `focus`).
    pub fn run(&self, focus: Option<&str>, deeplink: Option<DeepLink>, steps: Vec<Step>) -> Ran {
        let store = Store::open(&self.db, self.timeout).unwrap();
        let (back_tx, back_rx) = channel();
        let marked = Arc::clone(&self.marked);
        let marks = Marks::start(move |p| marked.lock().unwrap().push(p.map(str::to_string)));
        let sender = Sender::new(
            self.db.clone(),
            self.timeout,
            focus.map(str::to_string),
            Arc::clone(&self.handoff) as Arc<dyn Handoff>,
            marks.clone(),
            back_tx.clone(),
        );
        let snapshot = read(&store, &Shown::default()).unwrap();
        let parts = Parts {
            store,
            sender,
            agents: Arc::clone(&self.agents) as Arc<dyn Agents>,
            marks,
            back_tx,
            back_rx,
            radar_missing: Arc::new(AtomicBool::new(false)),
            theme: Theme::DARK,
        };
        let open = Open {
            focus_pane: focus.map(str::to_string),
            snapshot,
            deeplink,
        };
        let clock = Rc::new(HeldClock::default());
        clock.advance(10_000);
        let mut screen = Script {
            steps: steps.into(),
            clock: Rc::clone(&clock),
            last: String::new(),
            frames: Vec::new(),
            waits: Vec::new(),
            started: None,
        };
        run(open, parts, &mut screen, &*clock).unwrap();
        Ran {
            waits: screen.waits,
        }
    }
}
