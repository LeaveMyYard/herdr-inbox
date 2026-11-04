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
//! The tools' bodies run behind [`Tools`]. So far ([`Unported`]) only what
//! needs neither the store nor herdr is ported: the argument checks and
//! `ask_operator`'s refusals. The rest follows in P5-F's next slices; until
//! the switch (P5-I) nothing runs this server.
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

pub mod args;
pub mod ask;
pub mod receipts;
pub mod retry;
pub mod server;
pub mod tools;

use serde_json::Value;

use args::Call;

/// The tools' bodies: one call, its arguments already checked. Blocking.
/// `Ok` is the result the agent reads (a receipt included); `Err` a tool
/// error's text (Python's exception message).
pub trait Tools: Send + Sync + 'static {
    /// Runs `call`.
    fn call(&self, call: Call) -> Result<Value, String>;
}

/// The bodies ported so far: `ask_operator`'s refusals. Every other call is a
/// tool error that says it is not ported yet.
pub struct Unported;

impl Tools for Unported {
    fn call(&self, call: Call) -> Result<Value, String> {
        if let Call::AskOperator(a) = &call
            && let Err(receipt) = ask::checks::items(a)
        {
            return Ok(receipt);
        }
        Err("not ported to the Rust server yet (P5-F)".into())
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
    let srv = server::Server {
        tools: std::sync::Arc::new(Unported),
    };
    rt.block_on(async move {
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
    })
}
