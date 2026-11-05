//! `herdr-inbox mcp`: the MCP stdio server agents ask the operator through
//! (Python `server.py`), with its six tools.
//!
//! - [`tools`]: the tools' names, text and schemas, word for word as the
//!   Python server lists them.
//! - [`args`]: a call's arguments checked as pydantic checks them.
//! - [`retry`]: the busy retry and the not-migrated receipt around every call.
//! - [`receipts`]: the receipts a tool answers with when it cannot work.
//! - [`ask`]: `ask_operator`, its checks so far.
//! - [`server`]: the rmcp adapter; nothing else sees rmcp.
//!
//! - [`caller`]: who is asking, per call.
//! - [`answers`]: the `check_answers` tool; [`mine`]: `list_my_questions`
//!   and `withdraw_question`; [`alerting`]: `alert_operator` and `withdraw_alert`.
//! - [`runtime`](mod@runtime): the bodies as the server runs them; [`effects`] and
//!   [`herald`]: what a call does to herdr afterwards, off the call.
//!
//! The tools' bodies run behind [`Tools`] ([`runtime::Runtime`]). Until the
//! switch (P5-I) nothing runs this server.
//!
//! Stated deviations from the Python server:
//! - a call the schema refuses lists every refused field and reason as
//!   pydantic does, without pydantic's error codes, inputs and links;
//! - an integer argument beyond `i64` is refused by the schema (Python
//!   accepts it and the store fails the call later: an error either way);
//! - FastMCP's JSON pre-pass leaves JSON text nested deeper than 128 levels a
//!   string (Python reads it as JSON up to its recursion limit);
//! - the protocol envelope is rmcp's: the capabilities, the 2026-07-28 cache
//!   fields, the order of a tool's own keys, and no per-result
//!   `_meta.io.modelcontextprotocol/serverInfo` (discovery carries it). JSON-RPC
//!   edges no client sends are answered as rmcp answers them.

pub mod alerting;
pub mod answers;
pub mod args;
pub mod ask;
pub mod caller;
pub mod effects;
pub mod herald;
pub mod mine;
pub mod open;
pub mod receipts;
pub mod retry;
pub mod runtime;
pub mod server;
pub mod tools;

use std::sync::Arc;

use herdr_agent_kit::herdr_api::blocking::Herdr;
use herdr_agent_kit::plugin_env::PluginEnv;
use serde_json::Value;

use crate::claude::Transcripts;
use args::Call;

/// The tools' bodies: one call, its arguments already checked. Blocking.
/// `Ok` is the result the agent reads (a receipt included); `Err` a tool
/// error's text (Python's exception message).
pub trait Tools: Send + Sync + 'static {
    /// Runs `call`.
    fn call(&self, call: Call) -> Result<Value, String>;
}

/// The runtime for this process's environment.
pub fn runtime(env: &PluginEnv) -> runtime::Runtime {
    use herdr_agent_kit::internal::plugin_paths::inbox;

    let herdr = || Herdr::from_env();
    // the plugin's settings are found on the effects' thread, on the first marker refresh
    let herald = herald::Live::new(herdr(), env.clone(), inbox::db_path(env));
    let effects = effects::Effects::start(Arc::new(herald));
    runtime::Runtime {
        env: caller::Env {
            pane_id: env.pane_id().map(str::to_owned),
            session_id: env.var("CLAUDE_CODE_SESSION_ID").map(str::to_owned),
            tab_id: env.var("HERDR_TAB_ID").map(str::to_owned),
            cwd: std::env::current_dir()
                .ok()
                .map(|d| d.to_string_lossy().into_owned()),
        },
        world: Box::new(caller::live::Live {
            herdr: herdr(),
            files: Transcripts::from_env(),
            state_dir: inbox::state_dir(env),
            db: inbox::db_path(env),
        }),
        files: Transcripts::from_env(),
        db: inbox::db_path(env),
        server: inbox::server_id(env).to_string_lossy().into_owned(),
        backoff: retry::Backoff::default(),
        effects: Arc::new(effects),
        // the delivery worker's start (Python `worker.ensure`, ported as `worker::spawn::ensure`)
        // spawns `herdr-inbox worker`, which this binary has no subcommand for until P5-E2/I:
        // wired with it, as the CLI's `ensure_worker`
        ensure_worker: Box::new(|| {}),
        sleep: std::thread::sleep,
    }
}

/// `herdr-inbox mcp` on stdin and stdout, until the client goes; the exit status.
pub fn run() -> i32 {
    use rmcp::ServiceExt;

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("herdr-inbox mcp: {e}");
            return 1;
        }
    };
    let tools = Arc::new(runtime(&PluginEnv::from_env()));
    let effects = Arc::clone(&tools.effects);
    let srv = server::Server { tools };
    let code = rt.block_on(async move {
        match srv.serve(rmcp::transport::stdio()).await {
            Ok(running) => {
                let _ = running.waiting().await;
                0
            }
            Err(e) => {
                eprintln!("herdr-inbox mcp: {e}");
                1
            }
        }
    });
    effects.drain(effects::DRAIN); // what is queued at exit still runs (bounded)
    code
}
