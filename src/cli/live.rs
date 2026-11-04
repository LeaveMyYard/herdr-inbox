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
use crate::service::Delivery;
use crate::service::own::Me;
use crate::store::{Question, Store};
use crate::surface::Surface;
use crate::surface::live::Live;
use crate::tabs::lock::LOCK_TIMEOUT;
use crate::tabs::settings::PLUGIN_ID;

/// Runs `cmd` in this process: stdout, stderr and the environment are the
/// process's own. The exit code.
pub fn main(cmd: &str, argv: &[String]) -> i32 {
    let env = PluginEnv::from_env();
    let db_path = inbox::db_path(&env);
    let state_dir = inbox::state_dir(&env);
    let herdr = Herdr::from_env();
    let files = Transcripts::from_env();
    let mut config =
        ConfigFile::resolve(&env, || herdr.plugin_config_dir(PLUGIN_ID).ok().flatten());
    let mut surface = Live::new(herdr.clone(), warn);
    let mut delivery = LiveDelivery {
        herdr,
        files: Transcripts::from_env(),
        state_dir: state_dir.clone(),
        server: inbox::server_id(&env).to_string_lossy().into_owned(),
        unreadable: Unreadable::after(UNREADABLE_NOTIFY_AFTER),
    };
    let session = std::env::var("CLAUDE_CODE_SESSION_ID").ok();
    let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
    let mut io = Io {
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
    files: Transcripts,
    state_dir: std::path::PathBuf,
    server: String,
    unreadable: Unreadable,
}

impl Delivery for LiveDelivery {
    fn ensure_worker(&mut self, server: Option<&str>) {
        crate::worker::process::ensure_worker(server);
    }

    fn deliver(&mut self, store: &Store, qs: &[Question]) -> rusqlite::Result<()> {
        let server = self.server.clone();
        let local = local_answers(store, qs, false, &server, &mut |s| self.ensure_worker(s))?;
        let mut surface = Live::new(self.herdr.clone(), warn);
        let mut notify = |title: &str, body: &str| surface.notify(title, body);
        let mut ctx = Ctx {
            store,
            herdr: &self.herdr,
            files: &self.files,
            state_dir: &self.state_dir,
            notify: &mut notify,
            unreadable: &mut self.unreadable,
            verify_wait: VERIFY_SECONDS,
        };
        deliver_batch(&mut ctx, &local, Mode::default(), None)
    }
}
