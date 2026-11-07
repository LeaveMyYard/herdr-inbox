//! The real things behind [`Io`]: this process's environment, the inbox
//! database, herdr over its socket, Claude Code's transcripts, and the
//! delivery pass ([`crate::delivery::run`]).

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::internal::plugin_paths::inbox;
use herdr_agent_kit::plugin_env::PluginEnv;
use herdr_agent_kit::plugin_env::config::ConfigFile;
use inbox_core::deliver::Mode;

use super::{Io, run};
use crate::claude::Transcripts;
use crate::delivery::run::{
    Ctx, UNREADABLE_NOTIFY_AFTER, Unreadable, VERIFY_SECONDS, deliver_batch,
};
use crate::delivery::send::local_answers;
use crate::delivery::terminal::{self, Terminals};
use crate::service::Delivery;
use crate::service::own::Me;
use crate::store::{Question, Store};
use crate::surface::Surface;
use crate::surface::live::Live;
use crate::surface::popup::{focused_pane, own_checkout};
use crate::tabs::lock::LOCK_TIMEOUT;
use crate::tabs::settings::PLUGIN_ID;
use crate::worker::spawn::{enter_own_session, start_sender};

/// Runs `cmd` in this process: stdout, stderr and the environment are the
/// process's own. The exit code.
pub fn main(cmd: &str, argv: &[String]) -> i32 {
    if cmd == "deliver" {
        enter_own_session(); // the detached sender outlives the pane that started it
    }
    let env = PluginEnv::from_env();
    let db_path = inbox::db_path(&env);
    let herdr_config = crate::radar::config_path(&env);
    let state_dir = inbox::state_dir(&env);
    let herdr = crate::trace::observed(Herdr::from_env());
    let files = Transcripts::from_env();
    let mut config =
        ConfigFile::resolve(&env, || herdr.plugin_config_dir(PLUGIN_ID).ok().flatten());
    let exe = std::env::current_exe().ok();
    let root = exe.as_deref().and_then(own_checkout);
    let mut surface = Live::new(herdr.clone(), warn).with_popup(root, focused_pane(&env));
    let mut delivery = LiveDelivery {
        core: Box::new(terminal::live(&env, herdr.clone())),
        herdr,
        files: Transcripts::from_env(),
        state_dir: state_dir.clone(),
        server: inbox::server_id(&env).to_string_lossy().into_owned(),
        unreadable: Unreadable::after(UNREADABLE_NOTIFY_AFTER),
        exe,
    };
    let session = std::env::var("CLAUDE_CODE_SESSION_ID").ok();
    let (mut input, mut out, mut err) = (std::io::stdin(), std::io::stdout(), std::io::stderr());
    let mut io = Io {
        input: &mut input,
        out: &mut out,
        err: &mut err,
        db_path: &db_path,
        state_dir: &state_dir,
        files: &files,
        me: Me {
            session: session.as_deref(),
            pane: env.pane_id(),
        },
        delivery: &mut delivery,
        herdr: &mut surface,
        config: &mut config,
        herdr_config: &herdr_config,
        lock_wait: LOCK_TIMEOUT,
        now: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0)),
    };
    run(cmd, argv, &mut io)
}

fn warn(msg: &str) {
    let _ = writeln!(std::io::stderr(), "inbox: {msg}");
}

/// Delivery in this process: the pass over the socket, verified here.
struct LiveDelivery {
    herdr: Herdr,
    core: Box<dyn Terminals>,
    files: Transcripts,
    state_dir: std::path::PathBuf,
    server: String,
    unreadable: Unreadable,
    exe: Option<std::path::PathBuf>,
}

impl Delivery for LiveDelivery {
    fn ensure_worker(&mut self, server: Option<&str>) {
        crate::worker::process::ensure_worker(server);
    }

    fn spawn_deliver(&mut self, qids: &[i64], force: bool) {
        // recorded, not yet claimed: this machine's worker retries them
        if let Err(err) = start_sender(self.exe.as_deref(), &self.state_dir, qids, force) {
            warn(&format!(
                "could not start the delivery; the worker will retry: {err}"
            ));
        }
    }

    fn deliver_as(
        &mut self,
        store: &Store,
        qs: &[Question],
        retry: bool,
        force: bool,
    ) -> rusqlite::Result<()> {
        let server = self.server.clone();
        let local = local_answers(store, qs, retry, &server, &mut |s| self.ensure_worker(s))?;
        let mut surface = Live::new(self.herdr.clone(), warn);
        let mut notify = |title: &str, body: &str| surface.notify(title, body);
        let mut ctx = Ctx {
            store,
            herdr: &self.herdr,
            core: &*self.core,
            files: &self.files,
            state_dir: &self.state_dir,
            notify: &mut notify,
            unreadable: &mut self.unreadable,
            verify_wait: VERIFY_SECONDS,
        };
        let mode = Mode {
            retry,
            force,
            ..Mode::default()
        };
        deliver_batch(&mut ctx, &local, mode, None)
    }
}
