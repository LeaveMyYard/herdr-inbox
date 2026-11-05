//! `herdr-inbox tui` in this process (Python `cli`'s `tui`, `InboxApp().run()`):
//! the real database, herdr, terminal and clock behind the loop.
//!
//! - [`herdr`]: where the operator is, and going to an agent.
//! - [`handoff`]: delivery's side of a send.

pub mod handoff;
pub mod herdr;

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use ::herdr_agent_kit::herdr_api::blocking::Herdr;
use ::herdr_agent_kit::internal::plugin_paths::inbox;
use ::herdr_agent_kit::plugin_env::PluginEnv;
use ::herdr_agent_kit::plugin_env::config::ConfigFile;
use inbox_core::popup::{Open, Severity, Snapshot};
use inbox_core::popup_view::Theme;

use crate::cli::BUSY_TIMEOUT;
use crate::deeplink::NONCE_ENV;
use crate::store::Store;
use crate::surface::live::Live;
use crate::surface::markers::Markers;
use crate::surface::popup::FOCUS_ENV;
use crate::tabs::lock::LOCK_TIMEOUT;
use crate::tabs::settings::{self, PLUGIN_ID};
use crate::tui::back::Back;
use crate::tui::marks::Marks;
use crate::tui::open::{focus_pane, take};
use crate::tui::run::{Parts, run};
use crate::tui::screen::SystemClock;
use crate::tui::send::{BUSY, Sender, failure};
use crate::tui::snapshot::{Shown, read};
use crate::tui::term::Term;
use crate::tui::toasts::Toast;
use handoff::LiveHandoff;
use herdr::{LiveAgents, focused_pane, invocation_pane};

static LOG: OnceLock<PathBuf> = OnceLock::new();

/// Appends a warning to the inbox's `worker.log` (the popup owns the
/// terminal; Python's popup sends its spawn errors there too).
pub fn log(msg: &str) {
    if let Some(path) = LOG.get()
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = writeln!(f, "inbox: {msg}");
    }
}

/// Runs the popup; the exit code.
pub fn main() -> i32 {
    let env = PluginEnv::from_env();
    let db_path = inbox::db_path(&env);
    let state_dir = inbox::state_dir(&env);
    let _ = LOG.set(state_dir.join("worker.log"));
    let herdr = crate::trace::observed(Herdr::from_env());
    // the popup migrates (Python `Store(migrate=True)`): only it and the worker do
    let store = match Store::open_migrating(&db_path, BUSY_TIMEOUT) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("inbox: {err}");
            return 1;
        }
    };
    let (back_tx, back_rx) = channel();
    let focus = focus_pane(invocation_pane(&env), std::env::var(FOCUS_ENV).ok(), || {
        focused_pane(&herdr)
    });
    let snapshot = read(&store, &Shown::default(), SystemTime::now()).unwrap_or_else(|err| {
        let _ = back_tx.send(Back::Toast(Toast::new(failure(&err), Severity::Warning)));
        Snapshot::default()
    });
    let nonce = std::env::var(NONCE_ENV).ok();
    let (_lock, link) = take(&store, &state_dir, nonce.as_deref());
    let marks = Marks::start(marker(env.clone(), db_path.clone(), herdr.clone()));
    let handoff = Arc::new(LiveHandoff {
        herdr: herdr.clone(),
        state_dir: state_dir.clone(),
        server: inbox::server_id(&env).to_string_lossy().into_owned(),
        exe: std::env::current_exe().ok(),
        env: env.clone(),
    });
    let answer_expiry = answer_expiry(&env, &herdr);
    handoff::ensure_in_background(&handoff);
    let radar_missing = Arc::new(AtomicBool::new(false));
    check_radar(env.clone(), herdr.clone(), Arc::clone(&radar_missing));
    let sender = Sender::new(
        db_path,
        BUSY_TIMEOUT,
        focus.clone(),
        handoff,
        marks.clone(),
        back_tx.clone(),
    );
    let parts = Parts {
        store,
        sender,
        agents: Arc::new(LiveAgents(herdr)),
        marks,
        back_tx,
        back_rx,
        answer_expiry,
        radar_missing,
        theme: Theme::DARK, // the popup runs on textual-dark, as Python's; a light theme comes later
    };
    let open = Open {
        focus_pane: focus,
        snapshot,
        deeplink: link,
    };
    let mut term = match Term::enter() {
        Ok(term) => term,
        Err(err) => {
            eprintln!("inbox: no terminal for the popup: {err}");
            return 1;
        }
    };
    let ran = run(open, parts, &mut term, &SystemClock::start());
    drop(term);
    match ran {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("inbox: {err}");
            1
        }
    }
}

/// One pane's marks, refreshed on the background thread with its own
/// connection, herdr client and config (Python `service.update_markers`).
fn marker(env: PluginEnv, db_path: PathBuf, herdr: Herdr) -> impl FnMut(Option<&str>) + Send {
    let mut config: Option<ConfigFile> = None;
    let mut store: Option<Store> = None;
    move |pane| {
        if store.is_none() {
            store = Store::open(&db_path, BUSY_TIMEOUT).ok();
        }
        let Some(store) = store.as_ref() else {
            return log(BUSY);
        };
        // asked of herdr here, never on the popup's thread
        let config = config.get_or_insert_with(|| {
            ConfigFile::resolve(&env, || herdr.plugin_config_dir(PLUGIN_ID).ok().flatten())
        });
        let mut surface = Live::new(herdr.clone(), log);
        let mut warn = |m: &str| log(m);
        Markers {
            store,
            db_path: &db_path,
            herdr: &mut surface,
            config,
            lock_wait: LOCK_TIMEOUT,
            warn: &mut warn,
        }
        .update(pane);
    }
}

/// `remote_answer_expiry_hours`, read once as the popup opens: asking herdr
/// where the config lives never happens on the popup's thread once it runs.
fn answer_expiry(env: &PluginEnv, herdr: &Herdr) -> Duration {
    let mut config = ConfigFile::resolve(env, || herdr.plugin_config_dir(PLUGIN_ID).ok().flatten());
    settings::read(&mut config, &mut |m: &str| log(m)).answer_expiry
}

/// Read-only, off the popup's thread (Python `check_radar`): herdr-radar is
/// installed but its sidebar lacks `?N`, so the popup shows the hint line.
fn check_radar(env: PluginEnv, herdr: Herdr, missing: Arc<AtomicBool>) {
    let run = move || {
        let mut warn = |m: &str| log(m);
        let mut config =
            ConfigFile::resolve(&env, || herdr.plugin_config_dir(PLUGIN_ID).ok().flatten());
        let hint = settings::read(&mut config, &mut warn).radar_hint;
        let mut surface = Live::new(herdr, log);
        let path = crate::radar::config_path(&env);
        if crate::radar::missing(&path, hint, &mut surface, &mut warn) {
            missing.store(true, Ordering::Relaxed);
        }
    };
    // a hint is never worth an error: a thread that cannot start shows none
    let _ = std::thread::Builder::new()
        .name("inbox-radar".into())
        .spawn(run);
}
